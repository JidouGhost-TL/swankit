use ws_sound::drivers::byte::ByteCPcm;

#[test]
fn mask_zero_pcm_uses_driver_field_order_and_nine_bytes() {
    let pcm = ByteCPcm::parse(&[0, 0xb7, 1, 0x34, 0x12, 0x78, 0x56, 0xbc, 0x9a])
        .expect("nine-byte mask-zero PCM descriptor must parse");
    assert_eq!(pcm.bank, 0xb7);
    assert_eq!(pcm.flags, 1);
    assert_eq!(pcm.offset, 0x1234);
    assert_eq!(pcm.segment, 0x5678);
    assert_eq!(pcm.length, 0x9abc);
    assert_eq!(pcm.rate, 12000);
    assert!(!pcm.repeat);
}

#[test]
fn pcm_full_control_operand_drives_additive_dma_flags() {
    use ws_sound::drivers::byte::pcm_se;
    for (repeat_flag, operand, expected) in [(1, 0x12, 0x92), (1, 0x06, 0x86), (0, 0x08, 0x90)] {
        let command = pcm_se(&[0x8f, 0, 0, 0, 0, 1, 0, repeat_flag, 0, operand, 0xff]).unwrap();
        // Control bits in the final operand must survive above the rate bits.
        let decoded = serde_json::to_value(command).unwrap();
        assert_eq!(decoded["dma_control"], expected);
        assert_eq!(decoded["control_operand"], operand);
    }
}
