//! Biquad（双二阶）均衡处理器：RBJ cookbook 系数，带延迟线状态。

use crate::fixed::FixedProcessor;
use std::f32::consts::PI;

/// Biquad 滤波器类型（RBJ Audio EQ Cookbook）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BiquadKind {
    LowPass,
    HighPass,
    BandPass,
    Peak,
    LowShelf,
    HighShelf,
}

/// 已计算好的 biquad 系数（direct form 1 transposed 的归一化形式）。
#[derive(Debug, Clone, Copy)]
pub struct BiquadParams {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl BiquadParams {
    /// 根据 RBJ 公式计算归一化系数。
    pub fn rbj(kind: BiquadKind, sample_rate: f32, freq_hz: f32, q: f32, gain_db: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0); // 峰值/架式的 A
        let w0 = 2.0 * PI * (freq_hz / sample_rate).clamp(1e-6, 0.499);
        let cos = w0.cos();
        let sin = w0.sin();
        let alpha = sin / (2.0 * q.max(1e-3));

        let (b0, b1, b2, a0, a1, a2) = match kind {
            BiquadKind::LowPass => {
                let b1 = 1.0 - cos;
                let b0 = b1 / 2.0;
                let b2 = b1 / 2.0;
                let a0 = 1.0 + alpha;
                (b0, b1, b2, a0, -2.0 * cos, 1.0 - alpha)
            }
            BiquadKind::HighPass => {
                let b1 = -(1.0 + cos);
                let b0 = (1.0 + cos) / 2.0;
                let b2 = (1.0 + cos) / 2.0;
                let a0 = 1.0 + alpha;
                (b0, b1, b2, a0, -2.0 * cos, 1.0 - alpha)
            }
            BiquadKind::BandPass => {
                let b0 = alpha;
                let b1 = 0.0;
                let b2 = -alpha;
                let a0 = 1.0 + alpha;
                (b0, b1, b2, a0, -2.0 * cos, 1.0 - alpha)
            }
            BiquadKind::Peak => {
                let a0 = 1.0 + alpha / a;
                (
                    1.0 + alpha * a,
                    -2.0 * cos,
                    1.0 - alpha * a,
                    a0,
                    -2.0 * cos,
                    1.0 - alpha / a,
                )
            }
            BiquadKind::LowShelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                let a0 = (a + 1.0) + (a - 1.0) * cos + sq;
                let b0 = a * ((a + 1.0) - (a - 1.0) * cos + sq);
                let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos);
                let b2 = a * ((a + 1.0) - (a - 1.0) * cos - sq);
                let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos);
                let a2 = (a + 1.0) + (a - 1.0) * cos - sq;
                (b0, b1, b2, a0, a1, a2)
            }
            BiquadKind::HighShelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                let a0 = (a + 1.0) - (a - 1.0) * cos + sq;
                let b0 = a * ((a + 1.0) + (a - 1.0) * cos + sq);
                let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos);
                let b2 = a * ((a + 1.0) + (a - 1.0) * cos - sq);
                let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos);
                let a2 = (a + 1.0) - (a - 1.0) * cos - sq;
                (b0, b1, b2, a0, a1, a2)
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

/// 有状态 biquad 处理器。延迟线跨 buffer 保存。
pub struct Biquad {
    params: BiquadParams,
    // 延迟线状态（每声道独立）。
    x1: Vec<f32>,
    x2: Vec<f32>,
    y1: Vec<f32>,
    y2: Vec<f32>,
    channels: usize,
}

impl Biquad {
    pub fn new(params: BiquadParams, channels: usize) -> Self {
        Self {
            params,
            x1: vec![0.0; channels],
            x2: vec![0.0; channels],
            y1: vec![0.0; channels],
            y2: vec![0.0; channels],
            channels,
        }
    }

    /// 更新系数（例如音量/频率变化时）。保留延迟线状态。
    pub fn set_params(&mut self, params: BiquadParams) {
        self.params = params;
    }
}

impl FixedProcessor for Biquad {
    fn process(&mut self, input: &[f32], output: &mut [f32], frames: usize) {
        let ch = self.channels;
        let p = self.params;
        for f in 0..frames {
            for c in 0..ch {
                let idx = f * ch + c;
                let x = input[idx];
                let y = p.b0 * x + p.b1 * self.x1[c] + p.b2 * self.x2[c]
                    - p.a1 * self.y1[c]
                    - p.a2 * self.y2[c];
                self.x2[c] = self.x1[c];
                self.x1[c] = x;
                self.y2[c] = self.y1[c];
                self.y1[c] = y;
                output[idx] = y;
            }
        }
    }

    fn reset(&mut self) {
        for v in self
            .x1
            .iter_mut()
            .chain(&mut self.x2)
            .chain(&mut self.y1)
            .chain(&mut self.y2)
        {
            *v = 0.0;
        }
    }

    fn set_param(&mut self, id: u32, _value: f32) {
        // 系数重算需要采样率 + 类型 + freq + q + gain 的组合，单参数更新在此
        // 不做处理；由更高层（audio-graph / engine）按需整体重算后调 set_params。
        let _ = id;
    }

    fn name(&self) -> &'static str {
        "Biquad"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::FixedProcessor;

    /// 低通滤波器应显著衰减远高于截止频率的信号。
    /// A low-pass filter should strongly attenuate signals far above the cutoff.
    #[test]
    fn lowpass_attenuates_high_freq() {
        // 1kHz 截止低通 @48kHz，输入 12kHz 正弦（远高于截止频率）。
        // 1kHz-cutoff low-pass @48kHz, 12kHz sine input (far above cutoff).
        let p = BiquadParams::rbj(BiquadKind::LowPass, 48_000.0, 1_000.0, 0.707, 0.0);
        let mut bp = Biquad::new(p, 1);

        let n = 9600; // 足够长让滤波器进入稳态。Long enough to reach steady state.
        let mut input = vec![0.0f32; n];
        for (i, s) in input.iter_mut().enumerate() {
            *s = (2.0 * PI * 12_000.0 * i as f32 / 48_000.0).sin();
        }

        // 分块喂入（512 帧/块），验证跨 buffer 状态保持。
        // Feed in chunks (512 frames each) to verify state persists across buffers.
        let mut out = vec![0.0f32; n];
        let chunk = 512;
        let mut written = 0usize;
        for c in input.chunks(chunk) {
            let mut o = vec![0.0f32; c.len()];
            bp.process(c, &mut o, c.len());
            out[written..written + c.len()].copy_from_slice(&o);
            written += c.len();
        }

        // 跳过前 2048 个样本（过渡态），取稳态峰值幅度。
        // Skip the first 2048 samples (transient), take steady-state peak amplitude.
        let steady = &out[2048..];
        let peak = steady.iter().fold(0.0f32, |acc, &v| acc.max(v.abs()));

        // 输入峰值 1.0，二阶低通在 12kHz 处应衰减约 40dB+，输出峰值应远小于输入。
        // Input peak is 1.0; a 2nd-order low-pass should attenuate ~40dB+ at 12kHz,
        // so the output peak should be far below the input.
        assert!(
            peak < 0.05,
            "预期低通强衰减，实际稳态峰值 peak={peak}（应 < 0.05）"
        );
    }

    /// 分块处理与一次性处理应产生一致结果（验证跨 buffer 状态保持）。
    /// Chunked and one-shot processing should agree (state persists across buffers).
    #[test]
    fn chunked_matches_oneshot() {
        let p = BiquadParams::rbj(BiquadKind::LowPass, 48_000.0, 2_000.0, 0.707, 0.0);
        let mut bp_chunked = Biquad::new(p, 1);
        let mut bp_oneshot = Biquad::new(p, 1);

        let n = 1000;
        let input: Vec<f32> = (0..n)
            .map(|i| (2.0 * PI * 500.0 * i as f32 / 48_000.0).sin())
            .collect();

        // 一次性处理。One-shot.
        let mut out_oneshot = vec![0.0f32; n];
        bp_oneshot.process(&input, &mut out_oneshot, n);

        // 分块处理。Chunked.
        let mut out_chunked = vec![0.0f32; n];
        let mut written = 0usize;
        for c in input.chunks(100) {
            let mut o = vec![0.0f32; c.len()];
            bp_chunked.process(c, &mut o, c.len());
            out_chunked[written..written + c.len()].copy_from_slice(&o);
            written += c.len();
        }

        // 两种方式输出应一致（浮点误差内）。
        // Both should agree within floating-point tolerance.
        for i in 0..n {
            assert!(
                (out_oneshot[i] - out_chunked[i]).abs() < 1e-5,
                "样本 {i} 不一致：oneshot={} chunked={}",
                out_oneshot[i],
                out_chunked[i]
            );
        }
    }
}
