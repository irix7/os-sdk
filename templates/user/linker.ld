/*
 * Static n32 user image for the IRIX 7 ELF loader.
 *
 * The loader reads at most 256 KiB from LBA 0 of SCSI target 2, maps every
 * PT_LOAD segment into kuseg, zero-fills the memory range beyond p_filesz and
 * enters at e_entry.  The image therefore starts at 0x1000_0000, has absolute
 * addressing (the target is +noabicalls), and must not contain an interpreter
 * or dynamic section.
 *
 * Entry first, so e_entry is the first byte of the first loadable segment.
 */

OUTPUT_FORMAT("elf32-ntradbigmips")
OUTPUT_ARCH(mips)
ENTRY(_start)

SECTIONS
{
  . = 0x10000000;

  .text : {
    KEEP(*(.text._start))
    *(.text)
    *(.text.*)
  }

  .rodata : {
    *(.rodata)
    *(.rodata.*)
  }

  .data : {
    *(.data)
    *(.data.*)
    *(.sdata)
    *(.sdata.*)
    *(.got)
    *(.got.*)
  }

  .bss (NOLOAD) : {
    *(.bss)
    *(.bss.*)
    *(.sbss)
    *(.sbss.*)
    *(COMMON)
  }

  /DISCARD/ : {
    *(.MIPS.abiflags)
    *(.reginfo)
    *(.pdr)
    *(.comment)
    *(.gnu.attributes)
    *(.mdebug*)
    *(.options*)
    *(.eh_frame)
  }
}
