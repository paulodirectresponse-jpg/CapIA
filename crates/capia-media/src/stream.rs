//! Sessão de decodificação **persistente** de vídeo (ADR-059): um processo do ffmpeg que decodifica
//! para frente a partir de um quadro lógico e entrega quadros RGBA8 crus **em ordem**, com
//! contrapressão (o filho bloqueia no pipe quando o consumidor não lê). Reaproveitada para
//! reprodução/scrub sequencial: pedir o próximo quadro não custa um processo novo.
//!
//! O índice (ordenado por PTS) diz qual é o quadro lógico de cada saída: o n-ésimo quadro entregue
//! é o `first + n`. Matar a sessão (`Drop`) mata o processo.

use crate::decode::{DecodeLimits, PixelFormat, RawFrame, absolute_ticks, ffmpeg_of, frame_len};
use crate::error::{MediaError, MediaErrorCode};
use crate::ffprobe::{checked_input_path, file_url_arg};
use crate::index::FrameIndex;
use capia_time::Ticks;
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Quadros que a fila entre o leitor e o consumidor comporta (além do pipe do SO).
const QUEUE_DEPTH: usize = 3;

pub struct FrameStream {
    child: Child,
    rx: Receiver<Result<Vec<u8>, MediaError>>,
    width: u32,
    height: u32,
    next: usize,
    end: usize,
    timeout: Duration,
    times: Vec<(i64, Ticks)>,
    finished: bool,
}

impl std::fmt::Debug for FrameStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameStream")
            .field("next", &self.next)
            .field("end", &self.end)
            .finish_non_exhaustive()
    }
}

impl FrameStream {
    /// Abre a sessão no quadro lógico `first`. `attempt`: 0 = keyframe anterior, 1 = o keyframe
    /// antes dele, ≥ 2 = do início do arquivo (mais lento, sempre alcança).
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        tc: &crate::toolchain::MediaToolchain,
        path: &Path,
        index: &FrameIndex,
        width: u32,
        height: u32,
        first: usize,
        attempt: u8,
        limits: &DecodeLimits,
    ) -> Result<Self, MediaError> {
        let ffmpeg = ffmpeg_of(tc)?;
        let len = frame_len(width, height, limits)?;
        if first >= index.len() {
            return Err(MediaError::new(
                MediaErrorCode::MediaFrameNotFound,
                format!("frame {first} is outside the index"),
            ));
        }
        let abs = checked_input_path(path)?;
        let kf = index.keyframe_before(first).unwrap_or(0);
        let start = match attempt {
            0 => Some(kf),
            1 => kf.checked_sub(1).and_then(|k| index.keyframe_before(k)),
            _ => None,
        };
        let mut args: Vec<OsString> = [
            "-v",
            "error",
            "-nostdin",
            "-noautorotate",
            "-protocol_whitelist",
            "file",
            "-seek_timestamp",
            "1",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        if let Some(k) = start {
            let t = absolute_ticks(index, index.entries()[k].pts);
            args.push("-ss".into());
            args.push(crate::decode::seconds_arg(t, false).into());
        }
        args.push("-copyts".into());
        args.push("-i".into());
        args.push(file_url_arg(&abs));
        let pf = index.entries()[first].pts;
        let remaining = index.len() - first;
        for a in [
            "-map".to_owned(),
            format!("0:{}", index.stream_index()),
            "-an".into(),
            "-sn".into(),
            "-vf".into(),
            format!("select=gte(pts\\,{pf})"),
            "-fps_mode".into(),
            "passthrough".into(),
            "-frames:v".into(),
            remaining.to_string(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "rgba".into(),
            "pipe:1".into(),
        ] {
            args.push(a.into());
        }
        let mut cmd = Command::new(ffmpeg);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let mut tries = 0;
        let mut child = loop {
            match cmd.spawn() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 50 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    return Err(MediaError::new(
                        MediaErrorCode::MediaBackendFailed,
                        format!("cannot start ffmpeg: {e}"),
                    ));
                }
                Ok(c) => break c,
            }
        };
        let Some(mut out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(MediaError::new(
                MediaErrorCode::MediaBackendFailed,
                "cannot capture the decoder output",
            ));
        };
        let (tx, rx) = mpsc::sync_channel::<Result<Vec<u8>, MediaError>>(QUEUE_DEPTH);
        std::thread::spawn(move || {
            loop {
                let mut buf = vec![0u8; len];
                let mut got = 0usize;
                while got < len {
                    match out.read(&mut buf[got..]) {
                        Ok(0) => break,
                        Ok(n) => got += n,
                        Err(_) => break,
                    }
                }
                if got == len {
                    if tx.send(Ok(buf)).is_err() {
                        return;
                    }
                } else {
                    if got > 0 {
                        let _ = tx.send(Err(MediaError::new(
                            MediaErrorCode::MediaDecodeFailed,
                            "the decoder returned a partial frame",
                        )));
                    }
                    return;
                }
            }
        });
        let times = (first..index.len())
            .map(|i| (index.entries()[i].pts, index.time_of(i).unwrap_or(Ticks(0))))
            .collect();
        Ok(Self {
            child,
            rx,
            width,
            height,
            next: first,
            end: index.len(),
            timeout: limits.timeout,
            times,
            finished: false,
        })
    }

    /// Índice lógico do próximo quadro que `next_frame` entregará.
    pub fn next_index(&self) -> usize {
        self.next
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Próximo quadro em ordem de apresentação; `None` ao fim do stream (ou do índice).
    /// `cancel()` é consultado a cada ≤ 20 ms; verdadeiro mata a sessão e devolve `MEDIA_CANCELLED`.
    pub fn next_frame(
        &mut self,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Option<RawFrame>, MediaError> {
        if self.finished || self.next >= self.end {
            self.finished = true;
            return Ok(None);
        }
        let started = Instant::now();
        loop {
            if cancel() {
                self.kill();
                return Err(MediaError::new(
                    MediaErrorCode::MediaCancelled,
                    "the decode session was cancelled and its process killed",
                ));
            }
            if started.elapsed() >= self.timeout {
                self.kill();
                return Err(MediaError::new(
                    MediaErrorCode::MediaProbeTimeout,
                    "the decode session produced no frame in time and was killed",
                ));
            }
            match self.rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Ok(bytes)) => {
                    let i = self.next;
                    let (pts, time) = self.times[i - (self.end - self.times.len())];
                    self.next += 1;
                    return Ok(Some(RawFrame {
                        width: self.width,
                        height: self.height,
                        stride: self.width as usize * 4,
                        pixel_format: PixelFormat::Rgba8,
                        pts,
                        index: i,
                        time,
                        bytes,
                    }));
                }
                Ok(Err(e)) => {
                    self.kill();
                    return Err(e);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.finished = true;
                    let _ = self.child.wait();
                    return Ok(None);
                }
            }
        }
    }

    fn kill(&mut self) {
        self.finished = true;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for FrameStream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
