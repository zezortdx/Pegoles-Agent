//! Newtype IDs. No magic strings for identity.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::from_str(s)?))
            }
        }
    };
}

id_type!(AgentId);
id_type!(ComputerId);
id_type!(TaskId);
id_type!(SessionId);
// Unique ID of one structured action execution (Phase 5 lifecycle).
id_type!(ActionId);
// Identity of one captured guest frame (Phase 5 observation).
id_type!(FrameId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_display_fromstr() {
        let id = TaskId::new();
        let s = id.to_string();
        let parsed: TaskId = s.parse().expect("parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn ids_are_unique() {
        assert_ne!(TaskId::new(), TaskId::new());
    }
}
