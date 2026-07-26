use std::{
    collections::HashMap,
    f32::consts::PI,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};

const MAX_EQ_BANDS: usize = 10;
const MIN_FREQUENCY_HZ: f32 = 20.0;
const MAX_FREQUENCY_HZ: f32 = 20_000.0;
const MIN_GAIN_DB: f32 = -18.0;
const MAX_GAIN_DB: f32 = 18.0;
const MIN_Q: f32 = 0.1;
const MAX_Q: f32 = 12.0;
const MIN_PREAMP_DB: f32 = -24.0;
const MAX_PREAMP_DB: f32 = 12.0;
const LIMITER_THRESHOLD_DB: f32 = -0.3;
const LIMITER_RELEASE_SECONDS: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqFilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqBandConfig {
    pub kind: EqFilterKind,
    pub frequency_hz: f32,
    pub gain_db: f32,
    pub q: f32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEqConfig {
    pub enabled: bool,
    pub preamp_db: f32,
    pub bands: Vec<EqBandConfig>,
    pub limiter_enabled: bool,
}

impl Default for SessionEqConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp_db: 0.0,
            bands: vec![
                EqBandConfig {
                    kind: EqFilterKind::LowShelf,
                    frequency_hz: 80.0,
                    gain_db: 0.0,
                    q: 0.707,
                    enabled: true,
                },
                EqBandConfig {
                    kind: EqFilterKind::Peaking,
                    frequency_hz: 250.0,
                    gain_db: 0.0,
                    q: 1.0,
                    enabled: true,
                },
                EqBandConfig {
                    kind: EqFilterKind::Peaking,
                    frequency_hz: 1_000.0,
                    gain_db: 0.0,
                    q: 1.0,
                    enabled: true,
                },
                EqBandConfig {
                    kind: EqFilterKind::Peaking,
                    frequency_hz: 4_000.0,
                    gain_db: 0.0,
                    q: 1.0,
                    enabled: true,
                },
                EqBandConfig {
                    kind: EqFilterKind::HighShelf,
                    frequency_hz: 12_000.0,
                    gain_db: 0.0,
                    q: 0.707,
                    enabled: true,
                },
            ],
            limiter_enabled: true,
        }
    }
}

impl SessionEqConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.preamp_db.is_finite() || !(MIN_PREAMP_DB..=MAX_PREAMP_DB).contains(&self.preamp_db)
        {
            return Err(format!(
                "EQ 前级增益必须在 {MIN_PREAMP_DB} dB 到 {MAX_PREAMP_DB} dB 之间"
            ));
        }
        if self.bands.len() > MAX_EQ_BANDS {
            return Err(format!("EQ 最多支持 {MAX_EQ_BANDS} 个频段"));
        }

        for (index, band) in self.bands.iter().enumerate() {
            let number = index + 1;
            if !band.frequency_hz.is_finite()
                || !(MIN_FREQUENCY_HZ..=MAX_FREQUENCY_HZ).contains(&band.frequency_hz)
            {
                return Err(format!(
                    "EQ 第 {number} 段频率必须在 {MIN_FREQUENCY_HZ} Hz 到 {MAX_FREQUENCY_HZ} Hz 之间"
                ));
            }
            if !band.gain_db.is_finite() || !(MIN_GAIN_DB..=MAX_GAIN_DB).contains(&band.gain_db) {
                return Err(format!(
                    "EQ 第 {number} 段增益必须在 {MIN_GAIN_DB} dB 到 {MAX_GAIN_DB} dB 之间"
                ));
            }
            if !band.q.is_finite() || !(MIN_Q..=MAX_Q).contains(&band.q) {
                return Err(format!(
                    "EQ 第 {number} 段 Q 值必须在 {MIN_Q} 到 {MAX_Q} 之间"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct SessionEqManager {
    configs: Arc<Mutex<HashMap<u32, SessionEqConfig>>>,
}

impl SessionEqManager {
    pub fn get(&self, pid: u32) -> SessionEqConfig {
        self.configs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&pid)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set(&self, pid: u32, config: SessionEqConfig) -> Result<SessionEqConfig, String> {
        if pid == 0 {
            return Err("系统声音暂不支持单独 EQ".to_string());
        }
        config.validate()?;
        self.configs
            .lock()
            .map_err(|_| "EQ 配置锁已损坏".to_string())?
            .insert(pid, config.clone());
        Ok(config)
    }

    pub fn reset(&self, pid: u32) -> SessionEqConfig {
        self.configs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&pid);
        SessionEqConfig::default()
    }
}

#[derive(Debug, Clone, Copy)]
struct BiquadCoefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoefficients {
    fn from_band(band: &EqBandConfig, sample_rate: f32) -> Self {
        let frequency = band
            .frequency_hz
            .clamp(MIN_FREQUENCY_HZ, sample_rate * 0.49);
        let omega = 2.0 * PI * frequency / sample_rate;
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * band.q);
        let amplitude = 10.0_f32.powf(band.gain_db / 40.0);

        let (b0, b1, b2, a0, a1, a2) = match band.kind {
            EqFilterKind::Peaking => (
                1.0 + alpha * amplitude,
                -2.0 * cos_omega,
                1.0 - alpha * amplitude,
                1.0 + alpha / amplitude,
                -2.0 * cos_omega,
                1.0 - alpha / amplitude,
            ),
            EqFilterKind::LowShelf => {
                let two_sqrt_a_alpha = 2.0 * amplitude.sqrt() * alpha;
                (
                    amplitude
                        * ((amplitude + 1.0) - (amplitude - 1.0) * cos_omega + two_sqrt_a_alpha),
                    2.0 * amplitude * ((amplitude - 1.0) - (amplitude + 1.0) * cos_omega),
                    amplitude
                        * ((amplitude + 1.0) - (amplitude - 1.0) * cos_omega - two_sqrt_a_alpha),
                    (amplitude + 1.0) + (amplitude - 1.0) * cos_omega + two_sqrt_a_alpha,
                    -2.0 * ((amplitude - 1.0) + (amplitude + 1.0) * cos_omega),
                    (amplitude + 1.0) + (amplitude - 1.0) * cos_omega - two_sqrt_a_alpha,
                )
            }
            EqFilterKind::HighShelf => {
                let two_sqrt_a_alpha = 2.0 * amplitude.sqrt() * alpha;
                (
                    amplitude
                        * ((amplitude + 1.0) + (amplitude - 1.0) * cos_omega + two_sqrt_a_alpha),
                    -2.0 * amplitude * ((amplitude - 1.0) + (amplitude + 1.0) * cos_omega),
                    amplitude
                        * ((amplitude + 1.0) + (amplitude - 1.0) * cos_omega - two_sqrt_a_alpha),
                    (amplitude + 1.0) - (amplitude - 1.0) * cos_omega + two_sqrt_a_alpha,
                    2.0 * ((amplitude - 1.0) - (amplitude + 1.0) * cos_omega),
                    (amplitude + 1.0) - (amplitude - 1.0) * cos_omega - two_sqrt_a_alpha,
                )
            }
        };

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

struct Biquad {
    coefficients: BiquadCoefficients,
    z1: Vec<f32>,
    z2: Vec<f32>,
}

impl Biquad {
    fn new(coefficients: BiquadCoefficients, channels: usize) -> Self {
        Self {
            coefficients,
            z1: vec![0.0; channels],
            z2: vec![0.0; channels],
        }
    }

    fn process(&mut self, sample: f32, channel: usize) -> f32 {
        let output = self.coefficients.b0 * sample + self.z1[channel];
        self.z1[channel] =
            self.coefficients.b1 * sample - self.coefficients.a1 * output + self.z2[channel];
        self.z2[channel] = self.coefficients.b2 * sample - self.coefficients.a2 * output;
        if output.is_finite() { output } else { 0.0 }
    }
}

pub struct ParametricEq {
    enabled: bool,
    channels: usize,
    preamp_gain: f32,
    filters: Vec<Biquad>,
    limiter: Option<PeakLimiter>,
}

impl ParametricEq {
    pub fn new(config: &SessionEqConfig, sample_rate: u32, channels: u16) -> Result<Self, String> {
        config.validate()?;
        if sample_rate == 0 || channels == 0 {
            return Err("EQ 采样率和声道数必须大于 0".to_string());
        }

        let filters = config
            .bands
            .iter()
            .filter(|band| band.enabled && band.gain_db.abs() > f32::EPSILON)
            .map(|band| {
                Biquad::new(
                    BiquadCoefficients::from_band(band, sample_rate as f32),
                    channels as usize,
                )
            })
            .collect();

        Ok(Self {
            enabled: config.enabled,
            channels: channels as usize,
            preamp_gain: 10.0_f32.powf(config.preamp_db / 20.0),
            filters,
            limiter: config
                .limiter_enabled
                .then(|| PeakLimiter::new(sample_rate)),
        })
    }

    pub fn process_interleaved(&mut self, samples: &mut [f32]) {
        if !self.enabled {
            return;
        }

        for frame in samples.chunks_mut(self.channels) {
            for (channel, sample) in frame.iter_mut().enumerate() {
                let mut value = *sample * self.preamp_gain;
                for filter in &mut self.filters {
                    value = filter.process(value, channel);
                }
                *sample = value;
            }
            if let Some(limiter) = &mut self.limiter {
                limiter.process_frame(frame);
            }
        }
    }
}

struct PeakLimiter {
    threshold: f32,
    gain: f32,
    release_coefficient: f32,
}

impl PeakLimiter {
    fn new(sample_rate: u32) -> Self {
        Self {
            threshold: 10.0_f32.powf(LIMITER_THRESHOLD_DB / 20.0),
            gain: 1.0,
            release_coefficient: (-1.0 / (LIMITER_RELEASE_SECONDS * sample_rate as f32)).exp(),
        }
    }

    fn process_frame(&mut self, frame: &mut [f32]) {
        let peak = frame
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max);
        let target_gain = if peak > self.threshold {
            self.threshold / peak
        } else {
            1.0
        };

        if target_gain < self.gain {
            self.gain = target_gain;
        } else {
            self.gain = target_gain + (self.gain - target_gain) * self.release_coefficient;
        }
        for sample in frame {
            *sample = (*sample * self.gain).clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frequency_hz: f32, sample_rate: u32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|frame| {
                let sample =
                    (2.0 * PI * frequency_hz * frame as f32 / sample_rate as f32).sin() * 0.1;
                [sample, sample]
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn disabled_eq_is_bit_exact_bypass() {
        let config = SessionEqConfig::default();
        let mut processor = ParametricEq::new(&config, 48_000, 2).unwrap();
        let mut samples = sine(1_000.0, 48_000, 2_048);
        let original = samples.clone();
        processor.process_interleaved(&mut samples);
        assert_eq!(samples, original);
    }

    #[test]
    fn peaking_band_boosts_its_center_frequency() {
        let mut config = SessionEqConfig {
            enabled: true,
            limiter_enabled: false,
            ..SessionEqConfig::default()
        };
        for band in &mut config.bands {
            band.enabled = false;
        }
        config.bands.push(EqBandConfig {
            kind: EqFilterKind::Peaking,
            frequency_hz: 1_000.0,
            gain_db: 6.0,
            q: 1.0,
            enabled: true,
        });

        let mut samples = sine(1_000.0, 48_000, 12_000);
        let input_rms = rms(&samples[4_000..]);
        let mut processor = ParametricEq::new(&config, 48_000, 2).unwrap();
        processor.process_interleaved(&mut samples);
        let output_rms = rms(&samples[4_000..]);
        assert!(output_rms / input_rms > 1.9);
    }

    #[test]
    fn limiter_prevents_clipping_after_preamp() {
        let config = SessionEqConfig {
            enabled: true,
            preamp_db: 12.0,
            ..SessionEqConfig::default()
        };
        let mut processor = ParametricEq::new(&config, 48_000, 2).unwrap();
        let mut samples = vec![0.9; 4_096];
        processor.process_interleaved(&mut samples);
        assert!(samples.iter().all(|sample| sample.abs() <= 1.0));
        assert!(samples.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn rejects_invalid_configuration() {
        let mut config = SessionEqConfig::default();
        config.bands[0].frequency_hz = f32::NAN;
        assert!(config.validate().is_err());

        let config = SessionEqConfig {
            preamp_db: 99.0,
            ..SessionEqConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
