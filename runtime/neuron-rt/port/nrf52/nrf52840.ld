/* Linker script of the neuron-rt nRF52840 port: the application is linked above the Adafruit
 * UF2 bootloader (which occupies the low flash), with the vector table first and data copied to
 * SRAM at start-up. Flash by copying the UF2 to the NANO33BLE drive after a double-tap of RESET. */
ENTRY(nrt_nrf_reset)

MEMORY
{
  FLASH (rx) : ORIGIN = 0x00026000, LENGTH = 0x000DA000
  RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 0x00040000
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
