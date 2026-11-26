//! The line input: the MPC One's capture device, read without waiting.

use super::*;

/// What the line input counted since it was opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineStats {
    pub periods: u64,
    pub frames: u64,
    /// Periods that had too little input (filled with silence), after the
    /// first few.
    pub short: u64,
    /// Frames dropped because the input ran ahead.
    pub dropped: u64,
    /// The input was not read in time and the stream restarted.
    pub overruns: u64,
    pub errors: u64,
}

pub struct LineIn {
    pcm: Option<alsa::Pcm>,
    raw: Vec<i32>,
    /// Interleaved stereo, a ring of FIFO frames.
    pub(crate) fifo: Vec<f32>,
    pub(crate) head: usize,
    pub(crate) len: usize,
    pub stats: LineStats,
    errors_in_row: u32,
}

impl Default for LineIn {
    fn default() -> Self {
        LineIn::closed()
    }
}

impl LineIn {
    pub fn closed() -> LineIn {
        LineIn {
            pcm: None,
            raw: vec![0; 2 * FIFO],
            fifo: vec![0.0; 2 * FIFO],
            head: 0,
            len: 0,
            stats: LineStats::default(),
            errors_in_row: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.pcm.is_some()
    }

    /// Open card `card`'s capture device at the stock settings and start
    /// it. On any refusal the device is closed again and nothing else
    /// changes; the error says what the card said.
    pub fn open(&mut self, card: u32) -> Result<String, String> {
        if self.is_open() {
            return Ok("line input: already on".to_string());
        }
        let path = format!("/dev/snd/pcmC{card}D0c");
        let pcm = alsa::Pcm::open_capture(&path)
            .map_err(|e| format!("line input: {path}: {e} (it stays off)"))?;
        let mut hw = alsa::HwParams::stock_capture();
        if let Err(e) = pcm.hw_params(&mut hw) {
            let mut any = alsa::HwParams::format_only();
            let allows = match pcm.refine(&mut any) {
                Ok(()) => format!(
                    "the card allows period {}, periods {}, buffer {}",
                    any.interval(alsa::PARAM_PERIOD_SIZE).describe(),
                    any.interval(alsa::PARAM_PERIODS).describe(),
                    any.interval(alsa::PARAM_BUFFER_SIZE).describe()
                ),
                Err(e2) => format!("asking what it allows failed too: {e2}"),
            };
            return Err(format!(
                "line input: {path}: the stock capture settings were refused: {e}; {allows} \
                                (closed; it stays off)"
            ));
        }
        let buffer = hw.interval(alsa::PARAM_BUFFER_SIZE).single().unwrap_or(0);
        if buffer != CAPTURE_BUFFER || !hw.mask_has(alsa::PARAM_FORMAT, alsa::FORMAT_S32_LE) {
            return Err(format!("line input: {path}: unexpected buffer {buffer} frames or format (closed; it stays off)"));
        }
        // Wake every period; never start by itself (start() below); stop
        // (an overrun) when the buffer is full.
        let mut sw = alsa::SwParams::new(PERIOD as u32, u32::MAX >> 1, buffer);
        pcm.sw_params(&mut sw)
            .map_err(|e| format!("line input: sw params: {e} (closed; it stays off)"))?;
        pcm.prepare()
            .map_err(|e| format!("line input: prepare: {e} (closed; it stays off)"))?;
        pcm.start()
            .map_err(|e| format!("line input: start: {e} (closed; it stays off)"))?;
        self.pcm = Some(pcm);
        self.head = 0;
        self.len = 0;
        self.stats = LineStats::default();
        self.errors_in_row = 0;
        Ok(format!("line input: on: {path}, rate {}, channels {}, period {} frames, buffer {buffer} frames",
                   hw.interval(alsa::PARAM_RATE).describe(), hw.interval(alsa::PARAM_CHANNELS).describe(),
                   hw.interval(alsa::PARAM_PERIOD_SIZE).describe()))
    }

    /// Stop and close the capture device.
    pub fn close(&mut self) -> String {
        let Some(pcm) = self.pcm.take() else {
            return "line input: already off".to_string();
        };
        let _ = pcm.drop_stream();
        drop(pcm);
        self.len = 0;
        format!("line input: off ({})", self.describe())
    }

    pub fn describe(&self) -> String {
        let s = &self.stats;
        format!(
            "{} periods, {} short, {} frames dropped, {} overruns, {} errors",
            s.periods, s.short, s.dropped, s.overruns, s.errors
        )
    }

    /// Interleaved S32 frames that arrived (the stream, or a test).
    pub fn push_s32(&mut self, interleaved: &[i32]) {
        let frames = interleaved.len() / 2;
        for f in 0..frames {
            if self.len == FIFO {
                self.head = (self.head + 1) % FIFO;
                self.len -= 1;
                self.stats.dropped += 1;
            }
            let at = (self.head + self.len) % FIFO;
            self.fifo[2 * at] = interleaved[2 * f] as f32 / 2_147_483_648.0;
            self.fifo[2 * at + 1] = interleaved[2 * f + 1] as f32 / 2_147_483_648.0;
            self.len += 1;
        }
        self.stats.frames += frames as u64;
    }

    /// Read what the stream has. A note comes back if the input closed
    /// itself after errors.
    fn pull(&mut self) -> Option<String> {
        for _ in 0..2 {
            let room = FIFO - self.len;
            let pcm = self.pcm.as_ref()?;
            if room == 0 {
                return None;
            }
            let read = pcm.read_frames(&mut self.raw[..2 * room]);
            match read {
                Ok(0) => return None,
                Ok(n) => {
                    self.errors_in_row = 0;
                    let n = (n as usize).min(room);
                    let raw = std::mem::take(&mut self.raw);
                    self.push_s32(&raw[..2 * n]);
                    self.raw = raw;
                }
                Err(e) if e.raw_os_error() == Some(alsa::EPIPE) => {
                    self.stats.overruns += 1;
                    if pcm.prepare().and_then(|_| pcm.start()).is_err() {
                        self.stats.errors += 1;
                        self.errors_in_row += 1;
                    }
                    return None;
                }
                Err(e) => {
                    self.stats.errors += 1;
                    self.errors_in_row += 1;
                    if self.errors_in_row > MAX_ERRORS {
                        let n = self.errors_in_row;
                        let closed = self.close();
                        return Some(format!("line input: read failed {n} times in a row ({e}); turned off: {closed}"));
                    }
                    return None;
                }
            }
        }
        None
    }

    /// One period of input into `left` and `right` (silence while off, and
    /// for whatever has not arrived). A note comes back if something went
    /// wrong that the log should say.
    pub fn period(
        &mut self,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> Option<String> {
        if !self.is_open() {
            left.fill(0.0);
            right.fill(0.0);
            return None;
        }
        let note = self.pull();
        self.stats.periods += 1;
        let take = self.len.min(PERIOD);
        for f in 0..take {
            let at = (self.head + f) % FIFO;
            left[f] = self.fifo[2 * at];
            right[f] = self.fifo[2 * at + 1];
        }
        left[take..].fill(0.0);
        right[take..].fill(0.0);
        self.head = (self.head + take) % FIFO;
        self.len -= take;
        // The first periods after the start may come before any input.
        if take < PERIOD && self.stats.periods > 4 {
            self.stats.short += 1;
        }
        if self.len > 3 * PERIOD {
            let drop = self.len - 2 * PERIOD;
            self.head = (self.head + drop) % FIFO;
            self.len -= drop;
            self.stats.dropped += drop as u64;
        }
        note
    }
}

/// The latency to make up for when nothing has been measured: the output
/// buffer (`periods` of 128 frames) plus one period of input. The
/// converters add an unknown amount; measure it with a cable from out to in.
pub fn estimated_latency(periods: u32) -> u32 {
    (periods + 1) * PERIOD as u32
}
