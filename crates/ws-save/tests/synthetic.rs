use formatkit_core::Endian;
use ws_save::{
    checksum::{Crc16, Fold, Rule, Unit},
    Birthday, EepromSize, Media, OwnerProfile, SaveImage,
};

#[test]
fn standard_crc_vectors_and_arithmetic_variants() {
    assert_eq!(Crc16::KERMIT.compute(b"123456789"), 0x2189);
    assert_eq!(Crc16::X25.compute(b"123456789"), 0x906e);
    for crc in [Crc16::KERMIT, Crc16::X25] {
        assert_eq!(crc.compute_table(b"123456789"), crc.compute(b"123456789"));
    }
    let rule = Rule {
        unit: Unit::Word(Endian::Little),
        fold: Fold::Xor,
        bits: 16,
        seed: 0x3c,
        xor_out: 0,
        offset_base: 0,
    };
    assert_eq!(rule.compute(&[1, 2, 3, 4]).unwrap(), 0x63e);
    assert!(rule.compute(&[1]).is_err());
    let sum = Rule {
        unit: Unit::Byte,
        fold: Fold::Sum,
        bits: 8,
        seed: 0,
        xor_out: 255,
        offset_base: 0,
    };
    assert_eq!(sum.compute(&[255, 2]).unwrap(), 254);
    assert_eq!(
        Rule {
            fold: Fold::RotateThenSum(1),
            xor_out: 0,
            ..sum
        }
        .compute(&[128, 1])
        .unwrap(),
        2
    );
}

#[test]
fn reflected_crc_adapter_keeps_custom_parameters_and_swap() {
    let input: Vec<u8> = (0..=255).collect();
    for polynomial in [0x8408, 0xa001, 0xf80f] {
        for swap_bytes in [false, true] {
            let crc = Crc16 {
                polynomial,
                seed: 0x52a1,
                xor_out: 0xd70c,
                swap_bytes,
            };
            for bytes in [&input[..0], &input[..1], &input[..31], &input[..]] {
                assert_eq!(crc.compute(bytes), crc.compute_table(bytes));
            }
        }
    }
}

#[test]
fn owner_and_word_bounds_preserve_surroundings() {
    let mut bytes = [0u8; 128];
    bytes[0x60..0x64].copy_from_slice(&[11, 12, 13, 37]);
    bytes[0x70..0x74].copy_from_slice(&[0x20, 0x00, 0x02, 0x29]);
    let image = SaveImage::parse(Media::InternalMono, &bytes).unwrap();
    let mut profile = OwnerProfile::parse(&image).unwrap();
    assert_eq!(profile.name, "ABC♥");
    assert_eq!(
        profile.birthday,
        Some(Birthday {
            year: 2000,
            month: 2,
            day: 29
        })
    );
    profile.gender = 2;
    profile.write(Media::InternalMono, &mut bytes).unwrap();
    let before = bytes;
    assert!(SaveImage::write_word(Media::InternalMono, &mut bytes, usize::MAX, 5).is_err());
    assert_eq!(before, bytes);
    assert!(SaveImage::parse(Media::CartridgeEeprom(EepromSize::Kbit8), &bytes).is_err());
    assert_eq!(
        SaveImage::parse(Media::InternalMono, &bytes)
            .unwrap()
            .word(0x38)
            .unwrap(),
        0x0020
    );
}

#[test]
fn compound_checks_and_permutations_have_independent_vectors() {
    use ws_save::checksum::{
        lane_sum_xor, negate, rolling_pair, rotate_sum_right, scramble_words, xor_sum8,
    };
    assert_eq!(xor_sum8(&[1, 2, 3]), 0x0600);
    assert_eq!(lane_sum_xor(&[1, 2, 3, 4]).unwrap(), 0x0604);
    assert!(lane_sum_xor(&[1]).is_err());
    assert_eq!(rolling_pair(&[1, 2, 3]), 0x1006);
    assert_eq!(rotate_sum_right(&[1, 0, 2, 0]).unwrap(), 0x6000);
    assert_eq!(negate(1, 16).unwrap(), 65535);
    assert!(negate(1, 0).is_err());
    assert_eq!(
        scramble_words(&[1, 2, 3, 4], &[1, 0], &[1, 2]).unwrap(),
        [2, 4, 3, 2]
    );
    assert!(scramble_words(&[1, 2, 3, 4], &[1, 1], &[0, 0]).is_err());
    assert!(scramble_words(&[1, 2, 3, 4], &[0, 2], &[0, 0]).is_err());
    let rule = Rule {
        unit: Unit::Word(Endian::Little),
        fold: Fold::WeightedOffset,
        bits: 16,
        seed: 0,
        xor_out: 0,
        offset_base: 1,
    };
    assert_eq!(rule.compute(&[2, 0, 3, 0]).unwrap(), 11);
}
