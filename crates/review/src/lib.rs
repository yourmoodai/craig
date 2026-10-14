pub mod assessment;
pub mod history;
#[cfg(feature = "judge")]
pub mod judge;
pub mod policy;
pub mod store;
#[cfg(feature = "judge")]
pub mod worker;

pub type ReviewResult<T> = Result<T, String>;
pub fn fail<T>(code: &str) -> ReviewResult<T> {
    Err(code.into())
}
pub const DAY_MS: i64 = 86_400_000;
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(feature = "judge")]
pub mod comparison;
#[cfg(feature = "judge")]
pub mod presets;

#[cfg(windows)]
pub mod windows;
