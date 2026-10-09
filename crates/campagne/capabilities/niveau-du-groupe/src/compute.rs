//! The computation of `NiveauDuGroupe@1`: integer arithmetic only.
//!
//! It receives the levels the read exposes and counts every one of them. It
//! never filters (which PCs count is the views' job) and never repairs a value.

/// Level and count, before `model` and `asOf` are attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Computed {
    pub level: Option<u8>,
    pub pc_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComputeError {
    /// A level outside 1–20: the view broke its contract. Carries the index,
    /// never the value.
    #[error("level at index {index} is outside 1–20")]
    LevelOutOfRange { index: usize },
    #[error("party level arithmetic overflowed")]
    Overflow,
}

pub const LEVEL_MIN: i64 = 1;
pub const LEVEL_MAX: i64 = 20;

/// `(2·sum + n) / (2·n)` in integer division. An empty list is a success with
/// no level, decided before any division.
///
/// Every level is checked before anything is summed: one bad level fails the
/// whole call, with no partial result.
pub fn compute(levels: &[i64]) -> Result<Computed, ComputeError> {
    let mut sum: u64 = 0;
    for (index, &raw) in levels.iter().enumerate() {
        if !(LEVEL_MIN..=LEVEL_MAX).contains(&raw) {
            return Err(ComputeError::LevelOutOfRange { index });
        }
        let level = u64::try_from(raw).map_err(|_| ComputeError::LevelOutOfRange { index })?;
        sum = sum.checked_add(level).ok_or(ComputeError::Overflow)?;
    }
    let pc_count = u64::try_from(levels.len()).map_err(|_| ComputeError::Overflow)?;
    if pc_count == 0 {
        return Ok(Computed {
            level: None,
            pc_count: 0,
        });
    }
    Ok(Computed {
        level: Some(round_half_up_mean(sum, pc_count)?),
        pc_count,
    })
}

/// The rounding step alone, so the overflow path is testable without a huge
/// slice. A zero `n` has no mean and is reported as `Overflow` too.
pub(crate) fn round_half_up_mean(sum: u64, n: u64) -> Result<u8, ComputeError> {
    let numerator = sum
        .checked_mul(2)
        .and_then(|doubled| doubled.checked_add(n))
        .ok_or(ComputeError::Overflow)?;
    let denominator = n.checked_mul(2).ok_or(ComputeError::Overflow)?;
    let level = numerator
        .checked_div(denominator)
        .ok_or(ComputeError::Overflow)?;
    u8::try_from(level).map_err(|_| ComputeError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(levels: &[i64]) -> Option<u8> {
        compute(levels).expect("in range").level
    }

    #[test]
    fn the_examples_of_the_rule() {
        assert_eq!(level(&[3, 4]), Some(4));
        assert_eq!(level(&[1, 1, 2]), Some(1));
        assert_eq!(level(&[7]), Some(7));
        assert_eq!(level(&[20; 20]), Some(20));
        assert_eq!(compute(&[3, 4]).unwrap().pc_count, 2);
    }

    #[test]
    fn an_empty_party_has_no_level_and_is_not_an_error() {
        assert_eq!(
            compute(&[]),
            Ok(Computed {
                level: None,
                pc_count: 0
            })
        );
    }

    #[test]
    fn duplicates_count_twice() {
        let computed = compute(&[5, 5]).unwrap();
        assert_eq!((computed.level, computed.pc_count), (Some(5), 2));
    }

    #[test]
    fn a_level_outside_the_range_is_a_typed_error_at_its_index() {
        for bad in [0, 21, -1, i64::MIN, i64::MAX] {
            assert_eq!(
                compute(&[bad]),
                Err(ComputeError::LevelOutOfRange { index: 0 })
            );
        }
        assert_eq!(
            compute(&[5, 21, 3]),
            Err(ComputeError::LevelOutOfRange { index: 1 })
        );
        // The first bad one is reported, even after good ones and before others.
        assert_eq!(
            compute(&[5, 4, 0, 99]),
            Err(ComputeError::LevelOutOfRange { index: 2 })
        );
    }

    #[test]
    fn the_error_never_carries_the_value() {
        let message = compute(&[123_456_789]).unwrap_err().to_string();
        assert!(!message.contains("123456789"), "{message}");
    }

    #[test]
    fn the_rounding_step_reports_overflow_and_never_panics() {
        assert_eq!(round_half_up_mean(u64::MAX, 1), Err(ComputeError::Overflow));
        assert_eq!(round_half_up_mean(1, u64::MAX), Err(ComputeError::Overflow));
        assert_eq!(
            round_half_up_mean(u64::MAX / 2, u64::MAX / 2),
            Err(ComputeError::Overflow)
        );
        // No mean without a PC.
        assert_eq!(round_half_up_mean(0, 0), Err(ComputeError::Overflow));
        // A result above a level does not fit the level type either.
        assert_eq!(round_half_up_mean(300, 1), Err(ComputeError::Overflow));
        assert_eq!(round_half_up_mean(7, 1), Ok(7));
    }

    #[test]
    fn a_very_large_party_is_not_capped() {
        let computed = compute(&vec![20; 100_000]).unwrap();
        assert_eq!((computed.level, computed.pc_count), (Some(20), 100_000));
    }

    /// Every multiset of 1 to 3 levels over 1..=20: the level sits between the
    /// extremes, the count is the length, and the order is irrelevant.
    #[test]
    fn bounds_count_and_order_hold_over_the_whole_small_domain() {
        let range = 1..=20i64;
        let check = |levels: &[i64]| {
            let computed = compute(levels).unwrap();
            let got = i64::from(computed.level.expect("non-empty"));
            let min = *levels.iter().min().unwrap();
            let max = *levels.iter().max().unwrap();
            assert!(min <= got && got <= max, "{levels:?} gave {got}");
            assert_eq!(computed.pc_count, levels.len() as u64);
            let mut reversed = levels.to_vec();
            reversed.reverse();
            assert_eq!(compute(&reversed).unwrap(), computed, "{levels:?}");
            let mut sorted = levels.to_vec();
            sorted.sort_unstable();
            assert_eq!(compute(&sorted).unwrap(), computed, "{levels:?}");
        };
        for a in range.clone() {
            check(&[a]);
            for b in range.clone() {
                check(&[a, b]);
                for c in range.clone() {
                    check(&[a, b, c]);
                    check(&[c, a, b]);
                    check(&[b, c, a]);
                }
            }
        }
    }

    /// The oracle: exact rational rounding, kept apart from the production path.
    #[test]
    fn the_integer_formula_matches_an_exact_half_up_oracle() {
        for a in 1..=20i64 {
            for b in 1..=20i64 {
                let sum = a + b;
                // round-half-up(sum / 2), by cases.
                let expected = if sum % 2 == 0 { sum / 2 } else { sum / 2 + 1 };
                assert_eq!(level(&[a, b]), Some(u8::try_from(expected).unwrap()));
            }
        }
    }
}
