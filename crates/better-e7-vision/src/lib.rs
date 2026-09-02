use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
    time::Instant,
};

use better_e7_core::{
    Detection, Frame, NormalizedRect, PixelFormat, PixelRect, RecognitionError, Recognizer,
    VideoSource, VideoSourceError,
};

pub struct ImageFileSource {
    path: PathBuf,
    next_frame_id: u64,
    pending: Option<Frame>,
}

pub struct ImageSequenceSource {
    paths: Vec<PathBuf>,
    next_index: usize,
    next_frame_id: u64,
    started: bool,
}

impl ImageSequenceSource {
    #[must_use]
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            paths: paths.into_iter().collect(),
            next_index: 0,
            next_frame_id: 0,
            started: false,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

impl ImageFileSource {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            next_frame_id: 0,
            pending: None,
        }
    }
}

impl VideoSource for ImageFileSource {
    fn start(&mut self) -> Result<(), VideoSourceError> {
        let frame = load_rgb_frame(&self.path, self.next_frame_id)
            .map_err(|error| VideoSourceError(error.to_string()))?;
        self.next_frame_id = self.next_frame_id.saturating_add(1);
        self.pending = Some(frame);
        Ok(())
    }

    fn try_latest_frame(&mut self) -> Result<Option<Frame>, VideoSourceError> {
        Ok(self.pending.take())
    }

    fn stop(&mut self) -> Result<(), VideoSourceError> {
        self.pending = None;
        Ok(())
    }
}

impl VideoSource for ImageSequenceSource {
    fn start(&mut self) -> Result<(), VideoSourceError> {
        self.next_index = 0;
        self.next_frame_id = 0;
        self.started = true;
        Ok(())
    }

    fn try_latest_frame(&mut self) -> Result<Option<Frame>, VideoSourceError> {
        if !self.started {
            return Err(VideoSourceError(
                "image sequence source has not started".to_owned(),
            ));
        }
        let Some(path) = self.paths.get(self.next_index) else {
            return Ok(None);
        };
        let frame = load_rgb_frame(path, self.next_frame_id)
            .map_err(|error| VideoSourceError(error.to_string()))?;
        self.next_index = self.next_index.saturating_add(1);
        self.next_frame_id = self.next_frame_id.saturating_add(1);
        Ok(Some(frame))
    }

    fn stop(&mut self) -> Result<(), VideoSourceError> {
        self.started = false;
        Ok(())
    }
}

pub struct TemplateMatcher {
    label: String,
    template: Frame,
    threshold: f32,
    roi: NormalizedRect,
}

#[derive(Default)]
pub struct RecognizerSet {
    recognizers: Vec<Box<dyn Recognizer>>,
}

impl RecognizerSet {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            recognizers: Vec::new(),
        }
    }

    pub fn add(&mut self, recognizer: impl Recognizer + 'static) {
        self.recognizers.push(Box::new(recognizer));
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.recognizers.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.recognizers.is_empty()
    }
}

impl Recognizer for RecognizerSet {
    fn recognize(&self, frame: &Frame) -> Result<Vec<Detection>, RecognitionError> {
        let mut detections = Vec::new();
        for recognizer in &self.recognizers {
            detections.extend(recognizer.recognize(frame)?);
        }
        Ok(detections)
    }
}

impl TemplateMatcher {
    pub fn new(
        label: impl Into<String>,
        template: Frame,
        threshold: f32,
        roi: NormalizedRect,
    ) -> Result<Self, TemplateMatcherError> {
        if template.format() != PixelFormat::Rgb8 {
            return Err(TemplateMatcherError::UnsupportedPixelFormat);
        }
        if template.width() == 0 || template.height() == 0 {
            return Err(TemplateMatcherError::EmptyTemplate);
        }
        if !(0.0..=1.0).contains(&threshold) {
            return Err(TemplateMatcherError::InvalidThreshold(threshold));
        }
        Ok(Self {
            label: label.into(),
            template,
            threshold,
            roi,
        })
    }

    pub fn from_path(
        label: impl Into<String>,
        path: impl AsRef<Path>,
        threshold: f32,
        roi: NormalizedRect,
    ) -> Result<Self, TemplateMatcherError> {
        let template = load_rgb_frame(path.as_ref(), 0).map_err(TemplateMatcherError::Asset)?;
        Self::new(label, template, threshold, roi)
    }
}

impl Recognizer for TemplateMatcher {
    fn recognize(&self, frame: &Frame) -> Result<Vec<Detection>, RecognitionError> {
        if frame.format() != PixelFormat::Rgb8 {
            return Err(RecognitionError(
                "template matching requires an RGB8 frame".to_owned(),
            ));
        }
        let roi = self
            .roi
            .to_pixels(frame.width(), frame.height())
            .map_err(|error| RecognitionError(error.to_string()))?;
        let Some((x, y, confidence)) = find_best_match(frame, &self.template, roi) else {
            return Ok(Vec::new());
        };
        if confidence < self.threshold {
            return Ok(Vec::new());
        }

        let frame_width = frame.width() as f32;
        let frame_height = frame.height() as f32;
        let bounds = NormalizedRect::new(
            x as f32 / frame_width,
            y as f32 / frame_height,
            x.saturating_add(self.template.width()) as f32 / frame_width,
            y.saturating_add(self.template.height()) as f32 / frame_height,
        )
        .map_err(|error| RecognitionError(error.to_string()))?;
        Ok(vec![Detection {
            label: self.label.clone(),
            confidence,
            center: bounds.center(),
            bounds,
        }])
    }
}

fn find_best_match(frame: &Frame, template: &Frame, roi: PixelRect) -> Option<(u32, u32, f32)> {
    if template.width() > roi.width || template.height() > roi.height {
        return None;
    }
    let max_x = roi.right().checked_sub(template.width())?;
    let max_y = roi.bottom().checked_sub(template.height())?;
    let scan_step = (frame.width() / 480).max(frame.height() / 270).max(1) as usize;
    let sample_step = (template.width() / 16).max(template.height() / 16).max(1) as usize;
    let mut best = (roi.x, roi.y, -1.0_f32);

    for y in (roi.y..=max_y).step_by(scan_step) {
        for x in (roi.x..=max_x).step_by(scan_step) {
            let score = match_score(frame, template, x, y, sample_step);
            if score > best.2 {
                best = (x, y, score);
            }
        }
    }

    let radius = scan_step as u32;
    let start_x = best.0.saturating_sub(radius).max(roi.x);
    let start_y = best.1.saturating_sub(radius).max(roi.y);
    let end_x = best.0.saturating_add(radius).min(max_x);
    let end_y = best.1.saturating_add(radius).min(max_y);
    for y in start_y..=end_y {
        for x in start_x..=end_x {
            let score = match_score(frame, template, x, y, sample_step);
            if score > best.2 {
                best = (x, y, score);
            }
        }
    }
    Some(best)
}

fn match_score(frame: &Frame, template: &Frame, x: u32, y: u32, step: usize) -> f32 {
    let frame_width = frame.width() as usize;
    let template_width = template.width() as usize;
    let mut difference = 0_u64;
    let mut channel_count = 0_u64;

    for template_y in (0..template.height() as usize).step_by(step) {
        let frame_y = y as usize + template_y;
        for template_x in (0..template.width() as usize).step_by(step) {
            let frame_x = x as usize + template_x;
            let frame_index = (frame_y * frame_width + frame_x) * 3;
            let template_index = (template_y * template_width + template_x) * 3;
            for channel in 0..3 {
                difference = difference.saturating_add(u64::from(
                    frame.pixels()[frame_index + channel]
                        .abs_diff(template.pixels()[template_index + channel]),
                ));
                channel_count = channel_count.saturating_add(1);
            }
        }
    }

    1.0 - difference as f32 / (channel_count as f32 * 255.0)
}

fn load_rgb_frame(path: &Path, frame_id: u64) -> Result<Frame, RecognitionAssetError> {
    let image = image::open(path)
        .map_err(RecognitionAssetError::Image)?
        .into_rgb8();
    let (width, height) = image.dimensions();
    Frame::new(
        frame_id,
        Instant::now(),
        width,
        height,
        PixelFormat::Rgb8,
        image.into_raw(),
    )
    .map_err(RecognitionAssetError::Frame)
}

#[derive(Debug)]
pub enum RecognitionAssetError {
    Image(image::ImageError),
    Frame(better_e7_core::FrameError),
}

impl fmt::Display for RecognitionAssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image(error) => write!(formatter, "failed to load image: {error}"),
            Self::Frame(error) => write!(formatter, "failed to create image frame: {error}"),
        }
    }
}

impl Error for RecognitionAssetError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Image(error) => Some(error),
            Self::Frame(error) => Some(error),
        }
    }
}

#[derive(Debug)]
pub enum TemplateMatcherError {
    Asset(RecognitionAssetError),
    UnsupportedPixelFormat,
    EmptyTemplate,
    InvalidThreshold(f32),
}

impl fmt::Display for TemplateMatcherError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Asset(error) => write!(formatter, "failed to load template: {error}"),
            Self::UnsupportedPixelFormat => {
                formatter.write_str("template matching requires an RGB8 template")
            }
            Self::EmptyTemplate => formatter.write_str("template must not be empty"),
            Self::InvalidThreshold(value) => {
                write!(
                    formatter,
                    "template threshold must be within 0.0..=1.0: {value}"
                )
            }
        }
    }
}

impl Error for TemplateMatcherError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Asset(error) => Some(error),
            Self::UnsupportedPixelFormat | Self::EmptyTemplate | Self::InvalidThreshold(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::*;

    fn rgb_frame(width: u32, height: u32, pixels: Vec<u8>) -> Frame {
        Frame::new(0, Instant::now(), width, height, PixelFormat::Rgb8, pixels).unwrap()
    }

    #[test]
    fn finds_a_template_in_a_generated_frame() {
        let template_pixels = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0];
        let template = rgb_frame(2, 2, template_pixels.clone());
        let mut frame_pixels = vec![0_u8; 8 * 6 * 3];
        for row in 0..2 {
            let destination = ((2 + row) * 8 + 3) * 3;
            let source = row * 2 * 3;
            frame_pixels[destination..destination + 6]
                .copy_from_slice(&template_pixels[source..source + 6]);
        }
        let frame = rgb_frame(8, 6, frame_pixels);
        let matcher =
            TemplateMatcher::new("target", template, 0.99, NormalizedRect::full()).unwrap();

        let detections = matcher.recognize(&frame).unwrap();

        assert_eq!(detections.len(), 1);
        assert_eq!(detections[0].label, "target");
        assert_eq!(detections[0].confidence, 1.0);
        assert_eq!(
            detections[0].bounds,
            NormalizedRect::new(3.0 / 8.0, 2.0 / 6.0, 5.0 / 8.0, 4.0 / 6.0).unwrap()
        );
    }

    #[test]
    fn respects_the_recognition_roi() {
        let template = rgb_frame(1, 1, vec![255, 255, 255]);
        let mut pixels = vec![0_u8; 4 * 4 * 3];
        pixels[(3 * 4 + 3) * 3..].fill(255);
        let frame = rgb_frame(4, 4, pixels);
        let roi = NormalizedRect::new(0.0, 0.0, 0.5, 0.5).unwrap();
        let matcher = TemplateMatcher::new("target", template, 0.99, roi).unwrap();

        assert!(matcher.recognize(&frame).unwrap().is_empty());
    }

    #[test]
    fn loads_a_saved_image_as_a_video_source() {
        let suffix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("better-e7-{suffix}.png"));
        image::RgbImage::from_raw(2, 1, vec![1, 2, 3, 4, 5, 6])
            .unwrap()
            .save(&path)
            .unwrap();
        let mut source = ImageFileSource::new(&path);

        source.start().unwrap();
        let frame = source.try_latest_frame().unwrap().unwrap();

        assert_eq!((frame.width(), frame.height()), (2, 1));
        assert_eq!(frame.pixels(), [1, 2, 3, 4, 5, 6]);
        assert!(source.try_latest_frame().unwrap().is_none());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn replays_extracted_frames_for_recognition_regression() {
        let suffix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let first_path = std::env::temp_dir().join(format!("better-e7-{suffix}-0.png"));
        let second_path = std::env::temp_dir().join(format!("better-e7-{suffix}-1.png"));
        image::RgbImage::from_raw(2, 1, vec![0, 0, 0, 0, 0, 0])
            .unwrap()
            .save(&first_path)
            .unwrap();
        image::RgbImage::from_raw(2, 1, vec![0, 0, 0, 255, 0, 0])
            .unwrap()
            .save(&second_path)
            .unwrap();
        let matcher = TemplateMatcher::new(
            "target",
            rgb_frame(1, 1, vec![255, 0, 0]),
            0.99,
            NormalizedRect::full(),
        )
        .unwrap();
        let mut source = ImageSequenceSource::new([first_path.clone(), second_path.clone()]);

        source.start().unwrap();
        let first = source.try_latest_frame().unwrap().unwrap();
        let second = source.try_latest_frame().unwrap().unwrap();

        assert_eq!(source.len(), 2);
        assert_eq!((first.id(), second.id()), (0, 1));
        assert!(matcher.recognize(&first).unwrap().is_empty());
        assert_eq!(matcher.recognize(&second).unwrap().len(), 1);
        assert!(source.try_latest_frame().unwrap().is_none());
        source.stop().unwrap();
        fs::remove_file(first_path).unwrap();
        fs::remove_file(second_path).unwrap();
    }

    #[test]
    fn combines_detections_from_multiple_recognizers() {
        let red = rgb_frame(1, 1, vec![255, 0, 0]);
        let green = rgb_frame(1, 1, vec![0, 255, 0]);
        let frame = rgb_frame(2, 1, vec![255, 0, 0, 0, 255, 0]);
        let mut recognizers = RecognizerSet::new();
        recognizers.add(TemplateMatcher::new("red", red, 0.99, NormalizedRect::full()).unwrap());
        recognizers
            .add(TemplateMatcher::new("green", green, 0.99, NormalizedRect::full()).unwrap());

        let detections = recognizers.recognize(&frame).unwrap();

        assert_eq!(recognizers.len(), 2);
        assert_eq!(detections.len(), 2);
        assert_eq!(detections[0].label, "red");
        assert_eq!(detections[1].label, "green");
    }
}
