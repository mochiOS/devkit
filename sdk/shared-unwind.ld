/* Keep the linker's normal read-only EH layout. The CRT iterates the bounded
 * section and registers each FDE with the executable's LLVM libunwind. */
SECTIONS {
    HIDDEN(__mochios_dso_eh_start = ADDR(.eh_frame));
    HIDDEN(__mochios_dso_eh_end = ADDR(.eh_frame) + SIZEOF(.eh_frame));
}
INSERT AFTER .text;
