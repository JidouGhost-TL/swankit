use swankit::Toolkit;
#[test]
fn product_save_code_one_sizes_observed_32_kib_media() {
    let mut b = vec![0; 65536];
    b[65520..].copy_from_slice(&[0xea, 0, 0, 0, 0xf0, 0, 1, 0, 2, 1, 0, 1, 0, 1, 0, 0]);
    assert_eq!(
        Toolkit::new().unwrap().info(&b).unwrap().save_bytes,
        Some(32768)
    );
}
