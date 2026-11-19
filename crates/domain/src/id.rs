use std::fmt;

macro_rules! string_id {
    ($($name:ident),* $(,)?) => {$(
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    )*};
}

macro_rules! number_id {
    ($($name:ident($ty:ty)),* $(,)?) => {$(
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub $ty);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    )*};
}

string_id!(StageId, PusherCharacterId, EnemyTypeId);
number_id!(
    GoalId(u16),
    BridgeId(u16),
    RailId(u16),
    GameSessionId(u64),
    EnemyId(u64)
);

/// メダル種のID（定義データで決まる。全89種、基本設計 3.13）。
/// 大量のメダルEntityが持つため、複製は参照カウントの加算だけで済む `Arc<str>` にする。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MedalId(std::sync::Arc<str>);

impl MedalId {
    /// 開始時に所持しているノーマルメダルのID（定義データで必須）
    pub const NORMAL: &str = "normal";

    pub fn new(value: impl AsRef<str>) -> Self {
        Self(std::sync::Arc::from(value.as_ref()))
    }

    pub fn normal() -> Self {
        Self::new(Self::NORMAL)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MedalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
