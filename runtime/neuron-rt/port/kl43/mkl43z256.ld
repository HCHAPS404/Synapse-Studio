/* Linker script of the neuron-rt Kinetis MKL43Z256VLH4 port: code runs from flash at
 * 0x00000000 with the vector table first, then the MANDATORY 16-byte flash-configuration field
 * pinned at offset 0x400 (unsecured; without it the part can lock), then .text. Data is copied
 * to SRAM at start-up. On Kinetis KL the 32 KB SRAM straddles 0x20000000 (8 KB SRAM_L below,
 * 24 KB SRAM_U above), so the RAM region starts at 0x1FFFE000; the stack top is 0x20006000.
 * Flash by copying app.bin to the OpenSDA mass-storage drive. */
ENTRY(nrt_kl_reset)

MEMORY
{
  FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 256K
  RAM (rwx)  : ORIGIN = 0x1FFFE000, LENGTH = 0x8000
}

SECTIONS
{
  .text :
  {
    KEEP(*(.vectors))
    /* The flash configuration field must land at absolute 0x400. */
    . = 0x400;
    KEEP(*(.flash_config))
    *(.text*)
    *(.rodata*)
    . = ALIGN(4);
  } > FLASH

  .data :
  {
    __data_start__ = .;
    *(.data*)
    . = ALIGN(4);
    __data_end__ = .;
  } > RAM AT > FLASH
  __data_load__ = LOADADDR(.data);

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
