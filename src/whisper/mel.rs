use rustfft::{FftPlanner, num_complex::Complex};

const N_FFT: usize = 512;
const HOP: usize = 160;
const WIN: usize = 400;
pub const N_SAMPLES_30S: usize = 16000 * 30;

/// Returns mel spectrogram: [n_mel, n_frames], row-major.
/// `filters`: [n_mel, n_fft_bins] from GGML file (n_fft_bins = 257)
pub fn log_mel(samples: &[f32], filters: &[f32], n_mel: usize, n_fft_bins: usize) -> Vec<f32> {
    // Pad/trim to exactly 30 seconds
    let mut audio = samples.to_vec();
    audio.resize(N_SAMPLES_30S, 0.0);

    // Hann window
    let window: Vec<f32> = (0..WIN)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (WIN - 1) as f32).cos()))
        .collect();

    let n_frames = (N_SAMPLES_30S - WIN) / HOP + 1; // 2998, whisper pads to 3000

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);

    // power spectrum: [n_frames, n_fft_bins]
    let mut power = vec![0f32; n_frames * n_fft_bins];
    let mut buf = vec![Complex::new(0.0f32, 0.0); N_FFT];

    for t in 0..n_frames {
        let start = t * HOP;
        for i in 0..N_FFT {
            buf[i] = if i < WIN {
                Complex::new(audio[start + i] * window[i], 0.0)
            } else {
                Complex::new(0.0, 0.0)
            };
        }
        fft.process(&mut buf);
        for k in 0..n_fft_bins {
            power[t * n_fft_bins + k] = buf[k].norm_sqr();
        }
    }

    // Apply filters and log: mel[m, t] = log10(max(sum_k filter[m,k]*power[t,k], 1e-10))
    let mut mel = vec![0f32; n_mel * n_frames];
    for m in 0..n_mel {
        for t in 0..n_frames {
            let mut s = 0.0f32;
            for k in 0..n_fft_bins {
                s += filters[m * n_fft_bins + k] * power[t * n_fft_bins + k];
            }
            mel[m * n_frames + t] = s.max(1e-10).log10();
        }
    }

    // Normalize: clamp bottom, shift, scale
    let mmax = mel.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    for v in mel.iter_mut() {
        *v = ((*v).max(mmax - 8.0) + 4.0) / 4.0;
    }

    // Pad to exactly 3000 frames
    let mut out = vec![0f32; n_mel * 3000];
    for m in 0..n_mel {
        for t in 0..n_frames.min(3000) {
            out[m * 3000 + t] = mel[m * n_frames + t];
        }
    }
    out
}
