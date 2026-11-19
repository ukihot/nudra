//! プレイヤーに見せる経過時間の書式（`mm:ss.cc`）。
//!
//! 文字列リテラル禁止の例外: このファイルは数値の書式（`:` `.` と桁数）だけを持ち、
//! 言語に依存する文言を含まない。「プレイ時間」などの文言はカタログ（ui.json）に置く。

use std::{fmt, time::Duration};

/// ゲーム内の経過時間（一時停止を含まない）。1/100秒未満は切り捨てて表示する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlayTime(pub Duration);

impl fmt::Display for PlayTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let centis = self.0.as_millis() / 10;
        let minutes = centis / 6000;
        let seconds = centis % 6000 / 100;
        let hundredths = centis % 100;

        write!(f, "{minutes:02}:{seconds:02}.{hundredths:02}")
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::zero(Duration::ZERO, "00:00.00")]
    #[case::below_one_centi(Duration::from_millis(9), "00:00.00")]
    #[case::one_centi(Duration::from_millis(10), "00:00.01")]
    #[case::rounds_down(Duration::from_millis(1_016), "00:01.01")]
    #[case::one_minute(Duration::from_secs(60), "01:00.00")]
    #[case::mixed(Duration::from_millis(205_500), "03:25.50")]
    #[case::over_an_hour(Duration::from_secs(6_000), "100:00.00")]
    fn play_time_is_shown_as_minutes_seconds_centis(
        #[case] elapsed: Duration,
        #[case] shown: &str,
    ) {
        assert_eq!(PlayTime(elapsed).to_string(), shown);
    }

    #[rstest]
    fn longest_duration_does_not_panic() {
        assert!(!PlayTime(Duration::MAX).to_string().is_empty());
    }
}
