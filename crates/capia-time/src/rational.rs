use crate::ticks::TimeError;
use core::cmp::Ordering;
use core::fmt;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

/// Razão `num/den` sempre normalizada (`den > 0`, `gcd(|num|, den) = 1`). Usada para velocidade de
/// clip e taxas de quadros. Aritmética em `i128` com redução; overflow vira `TimeError`.
///
/// JSON: serializa como string `"n/d"` (ou `"n"` quando inteira); aceita inteiro, string `"n/d"` e
/// objeto `{ "num": n, "den": d }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rational {
    num: i64,
    den: i64,
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Rational {
    pub const ONE: Self = Self { num: 1, den: 1 };
    pub const ZERO: Self = Self { num: 0, den: 1 };

    /// Constrói e normaliza. `den == 0` é erro.
    pub fn new(num: i64, den: i64) -> Result<Self, TimeError> {
        Self::from_i128(i128::from(num), i128::from(den))
    }

    pub const fn from_int(n: i64) -> Self {
        Self { num: n, den: 1 }
    }

    fn from_i128(mut num: i128, mut den: i128) -> Result<Self, TimeError> {
        if den == 0 {
            return Err(TimeError::DivideByZero);
        }
        if den < 0 {
            num = -num;
            den = -den;
        }
        let g = gcd(num.unsigned_abs(), den.unsigned_abs());
        let g = i128::try_from(g.max(1)).map_err(|_| TimeError::Overflow)?;
        let (num, den) = (num / g, den / g);
        Ok(Self {
            num: i64::try_from(num).map_err(|_| TimeError::Overflow)?,
            den: i64::try_from(den).map_err(|_| TimeError::Overflow)?,
        })
    }

    pub const fn num(&self) -> i64 {
        self.num
    }

    pub const fn den(&self) -> i64 {
        self.den
    }

    pub const fn is_positive(&self) -> bool {
        self.num > 0
    }

    pub const fn is_integer(&self) -> bool {
        self.den == 1
    }

    pub fn checked_mul(self, other: Self) -> Result<Self, TimeError> {
        Self::from_i128(
            i128::from(self.num) * i128::from(other.num),
            i128::from(self.den) * i128::from(other.den),
        )
    }

    pub fn checked_div(self, other: Self) -> Result<Self, TimeError> {
        if other.num == 0 {
            return Err(TimeError::DivideByZero);
        }
        Self::from_i128(
            i128::from(self.num) * i128::from(other.den),
            i128::from(self.den) * i128::from(other.num),
        )
    }

    pub fn checked_add(self, other: Self) -> Result<Self, TimeError> {
        Self::from_i128(
            i128::from(self.num) * i128::from(other.den)
                + i128::from(other.num) * i128::from(self.den),
            i128::from(self.den) * i128::from(other.den),
        )
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, TimeError> {
        Self::from_i128(
            i128::from(self.num) * i128::from(other.den)
                - i128::from(other.num) * i128::from(self.den),
            i128::from(self.den) * i128::from(other.den),
        )
    }

    /// Inverso multiplicativo. Erro se `num == 0`.
    pub fn recip(self) -> Result<Self, TimeError> {
        Self::new(self.den, self.num)
    }

    /// Parse de `"n"` ou `"n/d"`.
    pub fn parse(s: &str) -> Result<Self, TimeError> {
        let s = s.trim();
        let (n, d) = match s.split_once('/') {
            Some((n, d)) => (n.trim(), d.trim()),
            None => (s, "1"),
        };
        let n: i64 = n.parse().map_err(|_| TimeError::InvalidRatio)?;
        let d: i64 = d.parse().map_err(|_| TimeError::InvalidRatio)?;
        Self::new(n, d)
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> Ordering {
        // den > 0 em ambos: compara em produto cruzado, sem overflow (i128).
        (i128::from(self.num) * i128::from(other.den))
            .cmp(&(i128::from(other.num) * i128::from(self.den)))
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl Serialize for Rational {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Repr {
    Int(i64),
    Text(String),
    Obj { num: i64, den: i64 },
}

impl<'de> Deserialize<'de> for Rational {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let parsed = match Repr::deserialize(deserializer)? {
            Repr::Int(n) => Ok(Self::from_int(n)),
            Repr::Text(s) => Self::parse(&s),
            Repr::Obj { num, den } => Self::new(num, den),
        };
        parsed.map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn new_normalises_sign_and_gcd() {
        let r = Rational::new(60_000, 2002).unwrap();
        assert_eq!((r.num(), r.den()), (30_000, 1001));
        let r = Rational::new(3, -6).unwrap();
        assert_eq!((r.num(), r.den()), (-1, 2));
        assert_eq!(Rational::new(1, 0), Err(TimeError::DivideByZero));
        assert_eq!(Rational::new(0, 5).unwrap(), Rational::ZERO);
    }

    #[test]
    fn arithmetic_is_exact() {
        let half = Rational::new(1, 2).unwrap();
        let third = Rational::new(1, 3).unwrap();
        assert_eq!(
            half.checked_add(third).unwrap(),
            Rational::new(5, 6).unwrap()
        );
        assert_eq!(
            half.checked_sub(third).unwrap(),
            Rational::new(1, 6).unwrap()
        );
        assert_eq!(
            half.checked_mul(third).unwrap(),
            Rational::new(1, 6).unwrap()
        );
        assert_eq!(
            half.checked_div(third).unwrap(),
            Rational::new(3, 2).unwrap()
        );
        assert_eq!(Rational::new(2, 1).unwrap().recip().unwrap(), half);
        assert_eq!(Rational::ZERO.recip(), Err(TimeError::DivideByZero));
    }

    #[test]
    fn ordering_does_not_overflow() {
        let a = Rational::new(i64::MAX, 2).unwrap();
        let b = Rational::new(i64::MAX - 1, 2).unwrap();
        assert!(a > b);
        assert!(Rational::new(1, 100).unwrap() < Rational::new(1, 99).unwrap());
    }

    #[test]
    fn parse_and_display_round_trip() {
        for s in ["1", "3/2", "1/100", "30000/1001", "-4/3"] {
            assert_eq!(Rational::parse(s).unwrap().to_string(), s);
        }
        assert_eq!(Rational::parse("6/4").unwrap().to_string(), "3/2");
        assert!(Rational::parse("x").is_err());
        assert!(Rational::parse("1/0").is_err());
    }

    #[test]
    fn json_accepts_int_string_and_object() {
        let two: Rational = serde_json::from_str("2").unwrap();
        assert_eq!(two, Rational::from_int(2));
        let r: Rational = serde_json::from_str("\"3/2\"").unwrap();
        assert_eq!(r, Rational::new(3, 2).unwrap());
        let r: Rational = serde_json::from_str("{\"num\":6,\"den\":4}").unwrap();
        assert_eq!(r, Rational::new(3, 2).unwrap());
        assert!(serde_json::from_str::<Rational>("\"1/0\"").is_err());
        assert_eq!(
            serde_json::to_string(&Rational::new(3, 2).unwrap()).unwrap(),
            "\"3/2\""
        );
    }
}
