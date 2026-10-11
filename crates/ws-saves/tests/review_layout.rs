use ws_saves::formats;
#[test]
fn bundled_fields_fit_their_declared_medium_and_edit_exact_extent() {
    for format in formats().unwrap() {
        for field in &format.fields {
            let label = field.medium.to_ascii_lowercase();
            let kind = if label.contains("internal")
                || label.contains("console")
                || label.contains("ieep")
            {
                "internal-eeprom"
            } else if label.contains("sram") {
                "cartridge-sram"
            } else {
                "cartridge-eeprom"
            };
            let media = format.definition()["media"].as_array().unwrap();
            let sizes: Vec<_> = media
                .iter()
                .filter(|m| m["kind"] == kind)
                .flat_map(|m| m["sizes"].as_array().unwrap())
                .map(|n| n.as_u64().unwrap() as usize)
                .collect();
            let capacity = *sizes.iter().max().expect("field medium must be declared");
            assert!(
                field.offset + field.size <= capacity,
                "{} {} extends past medium",
                format.id,
                field.name
            );
            if field.encoding == "bytes" || field.size > 4 {
                let mut image = vec![0xa5; capacity];
                let value = serde_json::json!(vec![0x12; field.size]);
                field.write(&mut image, &value).unwrap();
                assert_eq!(field.read(&image).unwrap(), value);
                assert!(image[..field.offset].iter().all(|b| *b == 0xa5));
                assert!(image[field.offset + field.size..]
                    .iter()
                    .all(|b| *b == 0xa5));
            }
        }
    }
}
