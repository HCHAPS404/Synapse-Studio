/* Linker script of the neuron-rt TM4C123GH6PM port: code in flash at 0x00000000 with the
 * vector table first, data copied to SRAM at start-up. FLASH 256 KB, SRAM 32 KB. */
ENTRY(nrt_tm4c_reset)

MEMORY
{
  FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 256K
  RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 32K
}

SECTIONS
{
  .text :
  {
    KEEP(*(.vectors))
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
