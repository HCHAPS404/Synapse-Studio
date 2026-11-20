(* neuron_kernel.st - state table interpreter of the Neuron micro-kernel (version 1).
   Table independent. The table is the word array of `<name>.nst.hex` as an
   ARRAY OF INT (a word above 32767 is written as its signed 16-bit value).

   VAR kern : NEURON_KERNEL; tbl : ARRAY [0..1023] OF INT; inp : ARRAY [0..31] OF INT; END_VAR
   ...                                  -- fill tbl once, then once per scan period:
   kern(TBL := tbl, LEN := <words>, INP := inp);
   IF kern.ERR = 0 THEN ... kern.CHG[i] <> 0 ... kern.OV[i] ... END_IF;

   CHG[i] is 1 when output i has to be written this scan, with value OV[i].
   ERR is 1 bad magic, 2 version, 3 size, 4 checksum. Values are 16-bit integers. *)
FUNCTION_BLOCK NEURON_KERNEL
VAR_INPUT
    TBL : ARRAY [0..1023] OF INT;
    LEN : INT;
    INP : ARRAY [0..31] OF INT;
END_VAR
VAR_OUTPUT
    OV : ARRAY [0..31] OF INT;
    CHG : ARRAY [0..31] OF INT;
    ERR : INT;
END_VAR
VAR
    started : BOOL;
    kst : INT;
    ktm : INT;
    vr : ARRAY [0..31] OF INT;
    okf : ARRAY [0..31] OF INT;
    pv : ARRAY [0..31] OF INT;
    pk : ARRAY [0..31] OF INT;
    stk : ARRAY [0..15] OF INT;
    nin, nout, nvar, nst, ntr, ini, flg : DINT;
    ovar, ost, otr, opr : DINT;
    sp, ph, ret, cp, pc, op, a, b, r, res : DINT;
    rounds, maxr, fired, ti, tr, gres, i, w, sum : DINT;
END_VAR

IF NOT started THEN
    ERR := 0;
    IF LEN < 12 THEN
        ERR := 3;
    ELSIF TBL[0] <> 20051 THEN
        ERR := 1;
    ELSIF TBL[1] <> 1 THEN
        ERR := 2;
    ELSIF TBL[10] <> LEN THEN
        ERR := 3;
    ELSE
        sum := 0;
        FOR i := 0 TO LEN - 1 DO
            IF i <> 11 THEN
                w := TBL[i];
                IF w < 0 THEN
                    w := w + 65536;
                END_IF;
                sum := (sum + w * (i + 1)) MOD 65521;
            END_IF;
        END_FOR;
        w := TBL[11];
        IF w < 0 THEN
            w := w + 65536;
        END_IF;
        IF w <> sum THEN
            ERR := 4;
        END_IF;
    END_IF;
    IF ERR <> 0 THEN
        RETURN;
    END_IF;
    flg := TBL[2];
    nin := TBL[3];
    nout := TBL[4];
    nvar := TBL[5];
    nst := TBL[6];
    ntr := TBL[7];
    ini := TBL[8];
    ovar := 12;
    ost := ovar + nvar;
    otr := ost + 5 * nst;
    opr := otr + 3 * ntr;
    FOR i := 0 TO 31 DO
        OV[i] := 0;
        CHG[i] := 0;
        okf[i] := 0;
        pv[i] := 0;
        pk[i] := 0;
        vr[i] := 0;
    END_FOR;
    FOR i := 0 TO nvar - 1 DO
        vr[i] := TBL[ovar + i];
    END_FOR;
    IF (flg AND 2) <> 0 THEN
        FOR i := 0 TO nout - 1 DO
            okf[i] := 1;
        END_FOR;
    END_IF;
    kst := ini;
    ktm := 0;
    started := TRUE;
    ph := 0;
ELSE
    ph := 1;
END_IF;

rounds := 0;
IF (flg AND 1) <> 0 THEN
    maxr := 64;
ELSE
    maxr := 1;
END_IF;

(* The scan is a small machine over the phase `ph`; phase 90 runs the block at `cp`
   and continues at `ret` (IEC has no subroutine that can reach these variables). *)
WHILE ph <> 99 DO
    CASE ph OF
        0:
            cp := TBL[ost + 5 * kst];
            ret := 1;
            ph := 90;
        1:
            fired := 0;
            ti := 0;
            cp := TBL[ost + 5 * kst + 2];
            ret := 2;
            ph := 90;
        2:
            IF ti >= TBL[ost + 5 * kst + 4] THEN
                ph := 8;
            ELSE
                tr := TBL[ost + 5 * kst + 3] + ti;
                cp := TBL[otr + 3 * tr + 1];
                IF cp = -1 THEN
                    gres := 1;
                    ph := 4;
                ELSE
                    ret := 3;
                    ph := 90;
                END_IF;
            END_IF;
        3:
            gres := res;
            ph := 4;
        4:
            IF gres <> 0 THEN
                cp := TBL[ost + 5 * kst + 1];
                ret := 5;
                ph := 90;
            ELSE
                ti := ti + 1;
                ph := 2;
            END_IF;
        5:
            cp := TBL[otr + 3 * tr + 2];
            ret := 6;
            ph := 90;
        6:
            kst := TBL[otr + 3 * tr];
            ktm := 0;
            fired := 1;
            cp := TBL[ost + 5 * kst];
            ret := 8;
            ph := 90;
        8:
            rounds := rounds + 1;
            IF fired = 0 OR rounds >= maxr THEN
                ph := 9;
            ELSE
                ph := 1;
            END_IF;
        9:
            IF ktm < 32767 THEN
                ktm := ktm + 1;
            END_IF;
            FOR i := 0 TO nout - 1 DO
                CHG[i] := 0;
                IF okf[i] <> 0 THEN
                    IF pk[i] = 0 OR pv[i] <> OV[i] THEN
                        CHG[i] := 1;
                    END_IF;
                    pv[i] := OV[i];
                    pk[i] := 1;
                END_IF;
            END_FOR;
            ph := 99;
        90:
            sp := 0;
            res := 0;
            IF cp <> -1 THEN
                pc := opr + cp;
                op := TBL[pc];
                WHILE op <> 0 DO
                    pc := pc + 1;
                    CASE op OF
                        1:
                            STK[sp] := TBL[pc];
                            sp := sp + 1;
                            pc := pc + 1;
                        2:
                            STK[sp] := INP[TBL[pc]];
                            sp := sp + 1;
                            pc := pc + 1;
                        3:
                            STK[sp] := vr[TBL[pc]];
                            sp := sp + 1;
                            pc := pc + 1;
                        4:
                            STK[sp] := OV[TBL[pc]];
                            sp := sp + 1;
                            pc := pc + 1;
                        5:
                            STK[sp] := ktm;
                            sp := sp + 1;
                        6:
                            sp := sp - 1;
                            OV[TBL[pc]] := STK[sp];
                            okf[TBL[pc]] := 1;
                            pc := pc + 1;
                        7:
                            sp := sp - 1;
                            vr[TBL[pc]] := STK[sp];
                            pc := pc + 1;
                        8:
                            IF STK[sp - 1] = 0 THEN
                                STK[sp - 1] := 1;
                            ELSE
                                STK[sp - 1] := 0;
                            END_IF;
                        20:
                            STK[sp - 1] := 0 - STK[sp - 1];
                        21:
                            ktm := 0;
                        9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19:
                            sp := sp - 1;
                            b := STK[sp];
                            a := STK[sp - 1];
                            r := 0;
                            CASE op OF
                                9:
                                    IF (a <> 0) AND (b <> 0) THEN
                                        r := 1;
                                    END_IF;
                                10:
                                    IF (a <> 0) OR (b <> 0) THEN
                                        r := 1;
                                    END_IF;
                                11:
                                    IF a = b THEN
                                        r := 1;
                                    END_IF;
                                12:
                                    IF a <> b THEN
                                        r := 1;
                                    END_IF;
                                13:
                                    IF a < b THEN
                                        r := 1;
                                    END_IF;
                                14:
                                    IF a <= b THEN
                                        r := 1;
                                    END_IF;
                                15:
                                    IF a > b THEN
                                        r := 1;
                                    END_IF;
                                16:
                                    IF a >= b THEN
                                        r := 1;
                                    END_IF;
                                17:
                                    r := a + b;
                                18:
                                    r := a - b;
                                19:
                                    r := a * b;
                            END_CASE;
                            STK[sp - 1] := r;
                    END_CASE;
                    op := TBL[pc];
                END_WHILE;
                IF sp > 0 THEN
                    res := STK[sp - 1];
                END_IF;
            END_IF;
            ph := ret;
    END_CASE;
END_WHILE;
END_FUNCTION_BLOCK
