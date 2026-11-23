/* Linker script of the neuron-rt STM32U585 port: code in flash with the vector table first,
 * data copied to SRAM at start-up. RAM is the contiguous SRAM1+SRAM2+SRAM3 (768 KB). */
ENTRY(nrt_u5_reset)

MEMORY
{
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 2048K
  RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 768K
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
