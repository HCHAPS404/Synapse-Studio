/* Linker script of the neuron-rt RP2040 / RP2350 port: the whole image lives in SRAM, with the
 * vector table first. Loaded by the boot ROM as a RAM UF2 (no flash loader needed). */
ENTRY(nrt_rp_reset)

MEMORY
{
  RAM (rwx) : ORIGIN = 0x20000000, LENGTH = 0x42000
}

SECTIONS
{
  .text :
  {
    KEEP(*(.vectors))
    *(.text*)
    *(.rodata*)
    . = ALIGN(4);
  } > RAM

  .data :
  {
    *(.data*)
    . = ALIGN(4);
  } > RAM

  .bss (NOLOAD) :
  {
    . = ALIGN(4);
    __bss_start__ = .;
    *(.bss*)
    *(COMMON)
    . = ALIGN(4);
    __bss_end__ = .;
  } > RAM

  __stack_top = ORIGIN(RAM) + LENGTH(RAM);
}
