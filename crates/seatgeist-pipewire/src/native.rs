use std::{
    os::fd::OwnedFd,
    sync::mpsc::{self, RecvTimeoutError, SyncSender},
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::{
    FrameSource, MAX_SOURCE_EDGE, PipeWireCaptureError, RawPixelFormat, RawVideoFrame, Result,
    latest_mailbox::{self, LatestReceiver, LatestSender},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipeWireStreamTarget {
    pub node_id: u32,
    pub pipewire_serial: Option<u64>,
}

enum WorkerEvent {
    Frame(RawVideoFrame),
    Error(String),
}

enum WorkerControl {
    Stop,
}

pub struct NativePipeWireFrameSource {
    events: LatestReceiver<WorkerEvent>,
    control: pipewire::channel::Sender<WorkerControl>,
    worker: Option<JoinHandle<()>>,
    closed: bool,
}

impl NativePipeWireFrameSource {
    pub fn open(fd: OwnedFd, target: PipeWireStreamTarget) -> Result<Self> {
        let (event_tx, event_rx) = latest_mailbox::channel();
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        let (control_tx, control_rx) = pipewire::channel::channel();
        let worker = thread::Builder::new()
            .name("seatgeist-pipewire".to_string())
            .spawn(move || run_pipewire_worker(fd, target, event_tx, startup_tx, control_rx))
            .map_err(|err| PipeWireCaptureError::Stream(format!("spawn worker: {err}")))?;
        match startup_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                events: event_rx,
                control: control_tx,
                worker: Some(worker),
                closed: false,
            }),
            Ok(Err(message)) => {
                let _ = worker.join();
                Err(PipeWireCaptureError::Stream(message))
            }
            Err(err) => {
                let _ = control_tx.send(WorkerControl::Stop);
                let _ = worker.join();
                Err(PipeWireCaptureError::Stream(format!(
                    "worker startup timed out: {err}"
                )))
            }
        }
    }
}

impl FrameSource for NativePipeWireFrameSource {
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<RawVideoFrame>> {
        if self.closed {
            return Err(PipeWireCaptureError::Stream(
                "PipeWire frame source is closed".to_string(),
            ));
        }
        match self.events.recv_timeout(timeout) {
            Ok(WorkerEvent::Frame(frame)) => Ok(Some(frame)),
            Ok(WorkerEvent::Error(message)) => Err(PipeWireCaptureError::Stream(message)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(PipeWireCaptureError::Stream(
                "PipeWire worker disconnected".to_string(),
            )),
        }
    }

    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let _ = self.control.send(WorkerControl::Stop);
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| {
                PipeWireCaptureError::Stream("PipeWire worker panicked".to_string())
            })?;
        }
        Ok(())
    }
}

impl Drop for NativePipeWireFrameSource {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn run_pipewire_worker(
    fd: OwnedFd,
    target: PipeWireStreamTarget,
    event_tx: LatestSender<WorkerEvent>,
    startup_tx: SyncSender<std::result::Result<(), String>>,
    control_rx: pipewire::channel::Receiver<WorkerControl>,
) {
    let startup_error = startup_tx.clone();
    let error_events = event_tx.clone();
    if let Err(err) = run_pipewire_worker_inner(fd, target, event_tx, startup_tx, control_rx) {
        let _ = startup_error.try_send(Err(err.to_string()));
        error_events.send(WorkerEvent::Error(err.to_string()));
    }
}

fn run_pipewire_worker_inner(
    fd: OwnedFd,
    target: PipeWireStreamTarget,
    event_tx: LatestSender<WorkerEvent>,
    startup_tx: SyncSender<std::result::Result<(), String>>,
    control_rx: pipewire::channel::Receiver<WorkerControl>,
) -> Result<()> {
    use pipewire as pw;
    use pw::spa;
    use spa::{
        param::format::{MediaSubtype, MediaType},
        pod::Pod,
    };

    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|err| PipeWireCaptureError::Stream(format!("create main loop: {err}")))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|err| PipeWireCaptureError::Stream(format!("create context: {err}")))?;
    let core = context
        .connect_fd_rc(fd, None)
        .map_err(|err| PipeWireCaptureError::Stream(format!("connect portal remote: {err}")))?;
    let mut properties = pipewire::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Video",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Screen",
    };
    if let Some(serial) = target.pipewire_serial {
        properties.insert(*pw::keys::TARGET_OBJECT, serial.to_string());
    }
    let stream = pw::stream::StreamBox::new(&core, "seatgeist-window-capture", properties)
        .map_err(|err| PipeWireCaptureError::Stream(format!("create stream: {err}")))?;
    let stop_loop = mainloop.clone();
    let _control_listener = control_rx.attach(mainloop.loop_(), move |control| match control {
        WorkerControl::Stop => stop_loop.quit(),
    });
    let data = WorkerData {
        format: spa::param::video::VideoInfoRaw::new(),
        format_ready: false,
        sequence: 0,
        event_tx: event_tx.clone(),
    };
    let error_tx = event_tx.clone();
    let _stream_listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(move |_, _, _, new| {
            if let pw::stream::StreamState::Error(message) = new {
                error_tx.send(WorkerEvent::Error(message));
            }
        })
        .param_changed(|_, user_data, id, param| {
            let Some(param) = param else {
                user_data.format_ready = false;
                return;
            };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = spa::param::format_utils::parse_format(param)
            else {
                return;
            };
            if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                return;
            }
            match user_data.format.parse(param) {
                Ok(_) => {
                    user_data.format_ready = raw_pixel_format(user_data.format.format()).is_ok()
                }
                Err(err) => {
                    user_data.format_ready = false;
                    user_data.event_tx.send(WorkerEvent::Error(format!(
                        "parse negotiated video format: {err}"
                    )));
                }
            }
        })
        .process(copy_latest_pipewire_buffer)
        .register()
        .map_err(|err| PipeWireCaptureError::Stream(format!("register stream listener: {err}")))?;

    let format_object = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::BGRA,
            spa::param::video::VideoFormat::RGBx,
            spa::param::video::VideoFormat::RGBA,
            spa::param::video::VideoFormat::BGR,
            spa::param::video::VideoFormat::RGB,
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                width: 1920,
                height: 1080
            },
            spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            spa::utils::Rectangle {
                width: MAX_SOURCE_EDGE,
                height: MAX_SOURCE_EDGE
            }
        ),
    );
    let serialized = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(format_object),
    )
    .map_err(|err| PipeWireCaptureError::Stream(format!("serialize format pod: {err}")))?
    .0
    .into_inner();
    let pod = Pod::from_bytes(&serialized)
        .ok_or_else(|| PipeWireCaptureError::Stream("construct format pod".to_string()))?;
    let mut params = [pod];
    stream
        .connect(
            spa::utils::Direction::Input,
            target.pipewire_serial.is_none().then_some(target.node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|err| PipeWireCaptureError::Stream(format!("connect capture stream: {err}")))?;
    let _ = startup_tx.try_send(Ok(()));
    mainloop.run();
    let _ = stream.disconnect();
    Ok(())
}

struct WorkerData {
    format: pipewire::spa::param::video::VideoInfoRaw,
    format_ready: bool,
    sequence: u64,
    event_tx: LatestSender<WorkerEvent>,
}

fn raw_pixel_format(format: pipewire::spa::param::video::VideoFormat) -> Result<RawPixelFormat> {
    use pipewire::spa::param::video::VideoFormat;
    match format {
        VideoFormat::BGRx => Ok(RawPixelFormat::Bgrx),
        VideoFormat::BGRA => Ok(RawPixelFormat::Bgra),
        VideoFormat::RGBx => Ok(RawPixelFormat::Rgbx),
        VideoFormat::RGBA => Ok(RawPixelFormat::Rgba),
        VideoFormat::BGR => Ok(RawPixelFormat::Bgr),
        VideoFormat::RGB => Ok(RawPixelFormat::Rgb),
        other => Err(PipeWireCaptureError::UnsupportedFormat(format!(
            "{other:?}"
        ))),
    }
}

fn copy_latest_pipewire_buffer(stream: &pipewire::stream::Stream, user_data: &mut WorkerData) {
    if !user_data.format_ready {
        return;
    }
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let Some(data) = buffer.datas_mut().first_mut() else {
        return;
    };
    if data
        .chunk()
        .flags()
        .contains(pipewire::spa::buffer::ChunkFlags::CORRUPTED)
    {
        return;
    }
    let chunk_size = usize::try_from(data.chunk().size()).unwrap_or(usize::MAX);
    let chunk_offset = usize::try_from(data.chunk().offset()).unwrap_or(usize::MAX);
    let stride = data.chunk().stride();
    let Some(mapped) = data.data() else {
        user_data.event_tx.send(WorkerEvent::Error(
            "PipeWire buffer is not mapped shared memory; DMA-BUF is not enabled".to_string(),
        ));
        return;
    };
    let size = chunk_size.min(mapped.len());
    if mapped.is_empty() || chunk_offset == usize::MAX {
        return;
    }
    let offset = chunk_offset % mapped.len();
    let mut owned = Vec::with_capacity(size);
    let first = size.min(mapped.len() - offset);
    owned.extend_from_slice(&mapped[offset..offset + first]);
    if first < size {
        owned.extend_from_slice(&mapped[..size - first]);
    }
    user_data.sequence = user_data.sequence.saturating_add(1);
    let size = user_data.format.size();
    let format = match raw_pixel_format(user_data.format.format()) {
        Ok(format) => format,
        Err(err) => {
            user_data.event_tx.send(WorkerEvent::Error(err.to_string()));
            return;
        }
    };
    let event = WorkerEvent::Frame(RawVideoFrame {
        width: size.width,
        height: size.height,
        stride,
        format,
        sequence: user_data.sequence,
        damage_present: false,
        data: owned,
    });
    user_data.event_tx.send(event);
}
