//! ユースケース（基本設計 4章）。Bevy・Avianに依存しない。
//! ユースケースは関数で、外側への指示は戻り値で返す。

mod evolve;
mod launch_medal;
mod progress;
mod resolve;
mod select_medal;
mod start_game;
mod waves;

pub use evolve::{EvolveOutput, evolve};
pub use launch_medal::{LaunchMedalError, LaunchOutput, launch_medal};
pub use progress::{
    ProgressStoreError, RecordResultError, load_progress, record_result, save_progress,
};
pub use resolve::{PhysicsFact, resolve_physics};
pub use select_medal::{SelectMedalError, select_medal};
pub use start_game::{
    PusherSetup, StartGameCommand, StartGameError, StartGameOutput, check_start, start_game,
};
pub use waves::{WaveOutput, advance_waves};

#[cfg(test)]
mod test_support;
