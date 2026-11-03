/* Augment ELF's complete default layout, retaining W^X, RELRO, dynamic
 * tables and TLS. No interpreter is used: the mochiOS exec producer loads the
 * complete, capability-validated DT_NEEDED closure. */
SECTIONS {
    HIDDEN(__eh_frame_start = ADDR(.eh_frame));
    HIDDEN(__eh_frame_end = ADDR(.eh_frame) + SIZEOF(.eh_frame));
    HIDDEN(__eh_frame_hdr_start = ADDR(.eh_frame_hdr));
    HIDDEN(__eh_frame_hdr_end = ADDR(.eh_frame_hdr) + SIZEOF(.eh_frame_hdr));
    HIDDEN(__mochios_data_start =
        MIN(SIZEOF(.tdata) ? ADDR(.tdata) : ADDR(.data),
        MIN(SIZEOF(.preinit_array) ? ADDR(.preinit_array) : ADDR(.data),
        MIN(SIZEOF(.init_array) ? ADDR(.init_array) : ADDR(.data),
            SIZEOF(.data.rel.ro) ? ADDR(.data.rel.ro) : ADDR(.data)))) & ~4095);
    HIDDEN(__mochios_data_end = ADDR(.bss) + SIZEOF(.bss));
}
INSERT AFTER .bss;
