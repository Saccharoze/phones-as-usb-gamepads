//! Sending Stones protocol: the single source of truth for the pad state,
//! the HID descriptor and the report bytes. Pure, no IO.

pub mod descriptor;
pub mod frame;
pub mod pad;
pub mod report;

pub use pad::{Button, Hat, PadState};
pub use frame::{ErrorCode, Frame, FrameError, Ping};
pub use report::{REPORT_LEN, ReportError};
