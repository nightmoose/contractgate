use serde::Serialize;

/// Whether to render output as human-readable text or machine-readable JSON.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Human,
    Json,
}

impl Mode {
    pub fn is_json(self) -> bool {
        self == Mode::Json
    }
}

/// Print a success line in human mode, or serialize `payload` in JSON mode.
pub fn ok<T: Serialize>(mode: Mode, human: &str, payload: &T) {
    if mode.is_json() {
        println!("{}", serde_json::to_string(payload).unwrap_or_default());
    } else {
        println!("{}", human);
    }
}

/// Print an error line in human mode, or serialize `payload` in JSON mode.
pub fn err<T: Serialize>(mode: Mode, human: &str, payload: &T) {
    if mode.is_json() {
        eprintln!("{}", serde_json::to_string(payload).unwrap_or_default());
    } else {
        eprintln!("{}", human);
    }
}
