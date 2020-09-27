use serde::{self, Deserialize};

pub(crate) mod string {
    use serde::de::{Deserializer, Error as DeError, Visitor};
    use std::{
        fmt::{Formatter, Result as FmtResult},
        marker::PhantomData,
    };

    struct IdVisitor<T: From<i64>>(PhantomData<T>);

    impl<'de, T: From<i64>> Visitor<'de> for IdVisitor<T> {
        type Value = T;

        fn expecting(&self, f: &mut Formatter<'_>) -> FmtResult {
            f.write_str("string or integer snowflake")
        }

        fn visit_i64<E: DeError>(self, value: i64) -> Result<Self::Value, E> {
            Ok(T::from(value))
        }

        fn visit_str<E: DeError>(self, value: &str) -> Result<Self::Value, E> {
            value.parse().map(T::from).map_err(DeError::custom)
        }
    }

    pub fn deserialize<'de, T: From<i64>, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<T, D::Error> {
        deserializer.deserialize_any(IdVisitor(PhantomData))
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UserId(#[serde(with = "string")] pub i64);
