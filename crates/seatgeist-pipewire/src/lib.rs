//! Bounded PipeWire ScreenCast frame consumption for Seatgeist.

use std::time::Duration;

use thiserror::Error;

mod encoding;
mod latest_mailbox;
mod native;
mod session;

pub use encoding::encode_bounded_png;
pub use native::{NativePipeWireFrameSource, PipeWireStreamTarget};
pub use session::{
    OpenedPortalCaptureSession, PipeWireCaptureSession, PortalPipeWireCaptureSession,
};

const MAX_SOURCE_EDGE: u32 = 16_384;
const MAX_SOURCE_BYTES: usize = 512 * 1024 * 1024;

pub type Result<T> = std::result::Result<T, PipeWireCaptureError>;

#[derive(Debug, Error)]
pub enum PipeWireCaptureError {
    #[error("unsupported PipeWire video format: {0}")]
    UnsupportedFormat(String),
    #[error("invalid PipeWire frame: {0}")]
    InvalidFrame(String),
    #[error("PipeWire stream failed: {0}")]
    Stream(String),
    #[error("PNG encoding failed: {0}")]
    Png(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPixelFormat {
    Bgrx,
    Bgra,
    Rgbx,
    Rgba,
    Bgr,
    Rgb,
}

impl RawPixelFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Bgrx | Self::Bgra | Self::Rgbx | Self::Rgba => 4,
            Self::Bgr | Self::Rgb => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawVideoFrame {
    pub width: u32,
    pub height: u32,
    pub stride: i32,
    pub format: RawPixelFormat,
    pub sequence: u64,
    pub damage_present: bool,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedFrame {
    pub png: Vec<u8>,
    pub source_width: u32,
    pub source_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub revision: String,
    pub sequence: u64,
    pub damage_present: bool,
}

/// Narrow ownership boundary for a retained PipeWire stream.
///
/// Implementations copy mapped shared-memory pixels into `RawVideoFrame`
/// before returning. DMA-BUF handles are never exposed through this trait.
pub trait FrameSource: Send {
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<RawVideoFrame>>;
    fn close(&mut self) -> Result<()>;
}
