/* Linker script of the neuron-rt SAMD21G18A port: the application is linked above the SAM-BA
 * bootloader (low 8 KB of flash), with the vector table first and data copied to SRAM at
 * start-up. Flash with bossac over the serial port (offset 0x2000). */
ENTRY(nrt_sd_reset)

MEMORY
{
  FLASH (rx) : ORIGIN = 0x00002000, LENGTH = 0x0003E000
  RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 0x00008000
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
