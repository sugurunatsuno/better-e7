use std::{
    error::Error,
    fmt,
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{self, JoinHandle},
    time::Instant,
};

use better_e7_core::{Frame, PixelFormat};

pub trait VideoDecoder: Send {
    fn push(&mut self, data: &[u8]) -> Result<(), VideoDecodeError>;
    fn try_next_frame(&mut self) -> Result<Option<Frame>, VideoDecodeError>;
}

pub trait VideoDecoderFactory: Send + Sync {
    fn create(&self) -> Result<Box<dyn VideoDecoder>, VideoDecodeError>;
}

#[derive(Debug, Clone)]
pub struct FfmpegProcessDecoderFactory {
    executable: PathBuf,
}

impl FfmpegProcessDecoderFactory {
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl VideoDecoderFactory for FfmpegProcessDecoderFactory {
    fn create(&self) -> Result<Box<dyn VideoDecoder>, VideoDecodeError> {
        FfmpegProcessDecoder::spawn(&self.executable)
            .map(|decoder| Box::new(decoder) as Box<dyn VideoDecoder>)
    }
}

pub struct FfmpegProcessDecoder {
    child: Child,
    stdin: Option<ChildStdin>,
    output_rx: Receiver<DecoderOutput>,
    reader_thread: Option<JoinHandle<()>>,
}

impl FfmpegProcessDecoder {
    pub fn spawn(executable: impl AsRef<Path>) -> Result<Self, VideoDecodeError> {
        let mut child = Command::new(executable.as_ref())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "h264",
                "-i",
                "pipe:0",
                "-an",
                "-f",
                "image2pipe",
                "-vcodec",
                "ppm",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(VideoDecodeError::Spawn)?;

        let stdin = child
            .stdin
            .take()
            .ok_or(VideoDecodeError::MissingPipe("stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(VideoDecodeError::MissingPipe("stdout"))?;
        let (output_tx, output_rx) = mpsc::channel();
        let reader_thread = thread::Builder::new()
            .name("better-e7-ffmpeg-output".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut frame_id = 0_u64;
                loop {
                    match read_ppm_frame(&mut reader, frame_id) {
                        Ok(Some(frame)) => {
                            frame_id = frame_id.saturating_add(1);
                            if output_tx.send(DecoderOutput::Frame(frame)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => {
                            let _ = output_tx.send(DecoderOutput::Finished);
                            break;
                        }
                        Err(error) => {
                            let _ = output_tx.send(DecoderOutput::Failed(error.to_string()));
                            break;
                        }
                    }
                }
            })
            .map_err(VideoDecodeError::ReaderThread)?;

        Ok(Self {
            child,
            stdin: Some(stdin),
            output_rx,
            reader_thread: Some(reader_thread),
        })
    }
}

impl VideoDecoder for FfmpegProcessDecoder {
    fn push(&mut self, data: &[u8]) -> Result<(), VideoDecodeError> {
        self.stdin
            .as_mut()
            .ok_or(VideoDecodeError::InputClosed)?
            .write_all(data)
            .map_err(VideoDecodeError::Write)
    }

    fn try_next_frame(&mut self) -> Result<Option<Frame>, VideoDecodeError> {
        match self.output_rx.try_recv() {
            Ok(DecoderOutput::Frame(frame)) => Ok(Some(frame)),
            Ok(DecoderOutput::Failed(message)) => Err(VideoDecodeError::Output(message)),
            Ok(DecoderOutput::Finished) | Err(TryRecvError::Disconnected) => {
                Err(VideoDecodeError::OutputClosed)
            }
            Err(TryRecvError::Empty) => Ok(None),
        }
    }
}

impl Drop for FfmpegProcessDecoder {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader_thread) = self.reader_thread.take() {
            let _ = reader_thread.join();
        }
    }
}

enum DecoderOutput {
    Frame(Frame),
    Failed(String),
    Finished,
}

fn read_ppm_frame(reader: &mut impl Read, frame_id: u64) -> io::Result<Option<Frame>> {
    let Some(magic) = read_ppm_token(reader)? else {
        return Ok(None);
    };
    if magic != "P6" {
        return Err(invalid_data(format!(
            "expected P6 PPM header, found {magic}"
        )));
    }

    let width = read_required_u32(reader, "width")?;
    let height = read_required_u32(reader, "height")?;
    let max_value = read_required_u32(reader, "maximum color value")?;
    if max_value != 255 {
        return Err(invalid_data(format!(
            "expected PPM maximum color value 255, found {max_value}"
        )));
    }

    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .and_then(|value| value.checked_mul(PixelFormat::Rgb8.bytes_per_pixel()))
        .ok_or_else(|| invalid_data("PPM dimensions overflow the address space"))?;
    let mut pixels = vec![0_u8; pixel_count];
    reader.read_exact(&mut pixels)?;
    Frame::new(
        frame_id,
        Instant::now(),
        width,
        height,
        PixelFormat::Rgb8,
        pixels,
    )
    .map(Some)
    .map_err(|error| invalid_data(error.to_string()))
}

fn read_required_u32(reader: &mut impl Read, name: &str) -> io::Result<u32> {
    let token = read_ppm_token(reader)?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::UnexpectedEof, format!("missing PPM {name}"))
    })?;
    token
        .parse()
        .map_err(|_| invalid_data(format!("invalid PPM {name}: {token}")))
}

fn read_ppm_token(reader: &mut impl Read) -> io::Result<Option<String>> {
    let mut token = Vec::new();
    let mut byte = [0_u8; 1];
    let mut in_comment = false;

    loop {
        match reader.read(&mut byte)? {
            0 if token.is_empty() => return Ok(None),
            0 => break,
            _ => {}
        }

        if in_comment {
            if byte[0] == b'\n' {
                in_comment = false;
            }
            continue;
        }
        if byte[0] == b'#' && token.is_empty() {
            in_comment = true;
            continue;
        }
        if byte[0].is_ascii_whitespace() {
            if token.is_empty() {
                continue;
            }
            break;
        }
        token.push(byte[0]);
    }

    String::from_utf8(token)
        .map(Some)
        .map_err(|error| invalid_data(error.to_string()))
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Debug)]
pub enum VideoDecodeError {
    Spawn(io::Error),
    ReaderThread(io::Error),
    MissingPipe(&'static str),
    InputClosed,
    Write(io::Error),
    Output(String),
    OutputClosed,
}

impl fmt::Display for VideoDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "failed to start FFmpeg: {error}"),
            Self::ReaderThread(error) => {
                write!(formatter, "failed to start FFmpeg output reader: {error}")
            }
            Self::MissingPipe(name) => write!(formatter, "FFmpeg did not provide its {name} pipe"),
            Self::InputClosed => formatter.write_str("FFmpeg input is closed"),
            Self::Write(error) => write!(formatter, "failed to write H.264 to FFmpeg: {error}"),
            Self::Output(message) => write!(formatter, "failed to read FFmpeg output: {message}"),
            Self::OutputClosed => formatter.write_str("FFmpeg output is closed"),
        }
    }
}

impl Error for VideoDecodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn(error) | Self::ReaderThread(error) | Self::Write(error) => Some(error),
            Self::MissingPipe(_) | Self::InputClosed | Self::Output(_) | Self::OutputClosed => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnexBUnit {
    pub data: Vec<u8>,
    pub nal_unit_type: u8,
}

#[derive(Debug, Default)]
pub struct AnnexBParser {
    buffer: Vec<u8>,
}

impl AnnexBParser {
    #[must_use]
    pub const fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<AnnexBUnit> {
        self.buffer.extend_from_slice(bytes);
        self.take_complete_units()
    }

    pub fn finish(&mut self) -> Option<AnnexBUnit> {
        let starts = find_start_codes(&self.buffer);
        let &(start, prefix_length) = starts.first()?;
        let header_index = start + prefix_length;
        if header_index >= self.buffer.len() {
            self.buffer.clear();
            return None;
        }
        let data = self.buffer.split_off(start);
        self.buffer.clear();
        Some(AnnexBUnit {
            nal_unit_type: data[prefix_length] & 0x1f,
            data,
        })
    }

    fn take_complete_units(&mut self) -> Vec<AnnexBUnit> {
        let starts = find_start_codes(&self.buffer);
        if starts.len() < 2 {
            return Vec::new();
        }

        let mut units = Vec::with_capacity(starts.len() - 1);
        for pair in starts.windows(2) {
            let (start, prefix_length) = pair[0];
            let (end, _) = pair[1];
            let header_index = start + prefix_length;
            if header_index < end {
                units.push(AnnexBUnit {
                    nal_unit_type: self.buffer[header_index] & 0x1f,
                    data: self.buffer[start..end].to_vec(),
                });
            }
        }

        let last_start = starts.last().map_or(0, |(start, _)| *start);
        self.buffer.drain(..last_start);
        units
    }
}

fn find_start_codes(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut starts = Vec::new();
    let mut index = 0;
    while index + 3 <= bytes.len() {
        if index + 4 <= bytes.len() && bytes[index..index + 4] == [0, 0, 0, 1] {
            starts.push((index, 4));
            index += 4;
        } else if bytes[index..index + 3] == [0, 0, 1] {
            starts.push((index, 3));
            index += 3;
        } else {
            index += 1;
        }
    }
    starts
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn reads_concatenated_ppm_frames() {
        let bytes = [
            b"P6\n# first frame\n2 1\n255\n".as_slice(),
            &[255, 0, 0, 0, 255, 0],
            b"P6\n1 1\n255\n".as_slice(),
            &[0, 0, 255],
        ]
        .concat();
        let mut reader = Cursor::new(bytes);

        let first = read_ppm_frame(&mut reader, 10).unwrap().unwrap();
        let second = read_ppm_frame(&mut reader, 11).unwrap().unwrap();

        assert_eq!((first.id(), first.width(), first.height()), (10, 2, 1));
        assert_eq!(first.pixels(), [255, 0, 0, 0, 255, 0]);
        assert_eq!((second.id(), second.width(), second.height()), (11, 1, 1));
        assert_eq!(second.pixels(), [0, 0, 255]);
        assert!(read_ppm_frame(&mut reader, 12).unwrap().is_none());
    }

    #[test]
    fn rejects_unsupported_ppm_data() {
        let mut reader = Cursor::new(b"P3\n1 1\n255\n0 0 0".as_slice());
        let result = read_ppm_frame(&mut reader, 0);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn extracts_units_across_chunk_boundaries() {
        let mut parser = AnnexBParser::new();
        assert!(parser.push(&[0, 0]).is_empty());
        assert!(parser.push(&[0, 1, 0x67, 1, 2, 0]).is_empty());

        let units = parser.push(&[0, 0, 0, 1, 0x68, 3]);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].nal_unit_type, 7);
        assert_eq!(units[0].data, [0, 0, 0, 1, 0x67, 1, 2]);

        let last = parser.finish().unwrap();
        assert_eq!(last.nal_unit_type, 8);
        assert_eq!(last.data, [0, 0, 0, 1, 0x68, 3]);
    }

    #[test]
    fn ignores_bytes_before_the_first_start_code() {
        let mut parser = AnnexBParser::new();
        let units = parser.push(&[9, 9, 0, 0, 1, 0x65, 7, 0, 0, 1, 0x61]);

        assert_eq!(units.len(), 1);
        assert_eq!(units[0].nal_unit_type, 5);
        assert_eq!(units[0].data, [0, 0, 1, 0x65, 7]);
    }
}
