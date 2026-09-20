//! Valve's 16-bit float, the storage the sound scripts' volume is kept in.
//!
//! The conversion is `float16` in `public/mathlib/compressed_vector.h` and is
//! reproduced bit for bit, including where it rounds toward zero and where it
//! disagrees with IEEE 754 on infinities and NaN. Volumes read back through
//! the sound emitter interface come from these bits, so a more accurate
//! conversion would be a louder or quieter game.

pub const MAX: f32 = 65504.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Float16(pub u16);

impl Float16 {
    pub fn from_f32(value: f32) -> Self {
        let value = value.clamp(-MAX, MAX);
        let bits = value.to_bits();
        let sign = (bits >> 31) & 1;
        let exponent = ((bits >> 23) & 0xff) as i32;
        let mantissa = bits & 0x007f_ffff;

        let (out_exponent, out_mantissa) = if exponent == 0 {
            (0, 0)
        } else if exponent == 0xff {
            /* Infinity becomes the largest finite value and NaN becomes zero,
            which is what the native conversion's enabled branches do. */
            if mantissa == 0 {
                (0x1e, 0x3ff)
            } else {
                (0, 0)
            }
        } else {
            let unbiased = exponent - 127;
            if unbiased < -14 {
                /* The native code tests `< -24` and then `< -14` without an
                else, so the first branch never survives; only the second
                assignment is kept here. */
                let shift = -14 - unbiased;
                if shift > 0 && shift < 11 {
                    (0, (1 << (10 - shift)) | (mantissa >> (13 + shift)))
                } else {
                    (0, 0)
                }
            } else if unbiased > 15 {
                (0x1e, 0x3ff)
            } else {
                ((unbiased + 15) as u32, mantissa >> 13)
            }
        };
        Self(((sign << 15) | (out_exponent << 10) | (out_mantissa & 0x3ff)) as u16)
    }

    pub fn to_f32(self) -> f32 {
        let sign = if self.0 >> 15 == 1 { -1.0 } else { 1.0 };
        let exponent = (self.0 >> 10) & 0x1f;
        let mantissa = self.0 & 0x3ff;
        if exponent == 31 {
            return if mantissa == 0 { MAX * sign } else { 0.0 };
        }
        if exponent == 0 {
            if mantissa == 0 {
                return 0.0;
            }
            return sign * (f32::from(mantissa) / 1024.0) * (1.0 / 16384.0);
        }
        let bits = (u32::from(self.0 >> 15) << 31)
            | ((u32::from(exponent) + (127 - 15)) << 23)
            | (u32::from(mantissa) << 13);
        f32::from_bits(bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_the_values_sound_scripts_use() {
        for volume in [0.0, 0.1, 0.25, 0.5, 0.7, 0.8, 1.0, 2.0] {
            let back = Float16::from_f32(volume).to_f32();
            assert!((back - volume).abs() < 0.001, "{volume} became {back}");
        }
        assert_eq!(Float16::from_f32(1.0).0, 0x3c00);
        assert_eq!(Float16::from_f32(-1.0).0, 0xbc00);
        assert_eq!(Float16::from_f32(0.0), Float16::default());
    }

    #[test]
    fn follows_the_native_conversion_where_it_is_unusual() {
        assert_eq!(Float16::from_f32(f32::NAN).0, 0);
        assert_eq!(Float16::from_f32(f32::INFINITY).0, 0x7bff);
        assert_eq!(Float16::from_f32(1e30).0, 0x7bff);
        assert_eq!(Float16(0x7bff).to_f32(), MAX);
        // Truncation, not rounding: the low mantissa bits are dropped.
        assert_eq!(Float16::from_f32(1.0 + 1.0 / 2048.0).0, 0x3c00);
    }
}
