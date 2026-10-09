//! Parametrised invertible XOR and bit-feedback rotation, selected by JSON.
use crate::value::bad;
use formatkit_core::Result;

pub(crate) fn word(value: u64, rounds: usize, params: &serde_json::Value) -> Result<u64> {
    let number = |key: &str| {
        params[key]
            .as_u64()
            .ok_or_else(|| bad("transform parameter"))
    };
    let bits = u32::try_from(number("bits")?).map_err(|_| bad("transform width"))?;
    let feedback = u32::try_from(number("feedback_bit")?).map_err(|_| bad("feedback bit"))?;
    let max_rounds = usize::try_from(number("max_rounds")?).map_err(|_| bad("transform rounds"))?;
    if !(2..=64).contains(&bits)
        || feedback >= bits - 1
        || max_rounds > 65536
        || rounds > max_rounds
    {
        return Err(bad("transform geometry or iteration limit"));
    }
    let mask = u64::MAX >> (64 - bits);
    let xor = number("xor_mask")?;
    if value > mask || xor > mask {
        return Err(bad("transform value outside word"));
    }
    let mut v = value;
    match params["direction"].as_str() {
        Some("encode") => {
            v ^= xor;
            for _ in 0..rounds {
                let clear = v & (1 << feedback) == 0;
                v = ((v << 1) | (v >> (bits - 1))) & mask;
                if clear {
                    v ^= 1;
                }
            }
        }
        Some("decode") => {
            for _ in 0..rounds {
                v = (v >> 1) | ((v & 1) << (bits - 1));
                if v & (1 << feedback) == 0 {
                    v ^= 1 << (bits - 1);
                }
            }
            v ^= xor;
        }
        _ => return Err(bad("transform direction")),
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    #[test]
    fn arbitrary_parameters_are_invertible_and_bounded() {
        for bits in [8, 16, 32, 64] {
            for feedback_bit in [0, 1, bits / 2] {
                let mut p = serde_json::json!({"bits":bits,"feedback_bit":feedback_bit,
                    "xor_mask":53,"max_rounds":63,"direction":"encode"});
                for n in 0..64 {
                    for value in [0, 1, 53, 64, 255] {
                        p["direction"] = "encode".into();
                        let encoded = super::word(value, n, &p).unwrap();
                        p["direction"] = "decode".into();
                        assert_eq!(super::word(encoded, n, &p).unwrap(), value);
                    }
                }
                assert!(super::word(0, 64, &p).is_err());
            }
        }
        assert!(super::word(0, 0, &serde_json::json!({"bits":0})).is_err());
    }
}
