// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//
// SIMD optimizations for audio processing
// Maintains compatibility with non-SIMD platforms through runtime feature detection

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;

/// SIMD-optimized audio mixing with runtime feature detection
/// Falls back to scalar implementation on non-SIMD platforms
pub fn mix_samples_simd(
    frame: &mut [f32],
    source_frame: &[f32],
    channel_mappings: &[Vec<usize>],
) {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe {
                mix_samples_avx2(frame, source_frame, channel_mappings);
                return;
            }
        }
        if is_x86_feature_detected!("sse4.1") {
            unsafe {
                mix_samples_sse4(frame, source_frame, channel_mappings);
                return;
            }
        }
    }
    
    #[cfg(target_arch = "aarch64")]
    {
        if is_aarch64_feature_detected!("neon") {
            unsafe {
                mix_samples_neon(frame, source_frame, channel_mappings);
                return;
            }
        }
    }
    
    // Fallback to scalar implementation for all platforms
    mix_samples_scalar(frame, source_frame, channel_mappings);
}

/// SIMD-optimized buffer clearing with runtime feature detection
pub fn clear_buffer_simd(buffer: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe {
                clear_buffer_avx2(buffer);
                return;
            }
        }
        if is_x86_feature_detected!("sse2") {
            unsafe {
                clear_buffer_sse2(buffer);
                return;
            }
        }
    }
    
    #[cfg(target_arch = "aarch64")]
    {
        if is_aarch64_feature_detected!("neon") {
            unsafe {
                clear_buffer_neon(buffer);
                return;
            }
        }
    }
    
    // Fallback to scalar implementation
    clear_buffer_scalar(buffer);
}

/// SIMD-optimized buffer copying with runtime feature detection
pub fn copy_buffer_simd(dst: &mut [f32], src: &[f32]) {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe {
                copy_buffer_avx2(dst, src);
                return;
            }
        }
        if is_x86_feature_detected!("sse2") {
            unsafe {
                copy_buffer_sse2(dst, src);
                return;
            }
        }
    }
    
    #[cfg(target_arch = "aarch64")]
    {
        if is_aarch64_feature_detected!("neon") {
            unsafe {
                copy_buffer_neon(dst, src);
                return;
            }
        }
    }
    
    // Fallback to scalar implementation
    copy_buffer_scalar(dst, src);
}

// ============================================================================
// AVX2 Implementations
// ============================================================================

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn mix_samples_avx2(
    frame: &mut [f32],
    source_frame: &[f32],
    channel_mappings: &[Vec<usize>],
) {
    // Process 8 samples at a time with AVX2
    const SIMD_WIDTH: usize = 8;
    
    for (source_channel, &sample) in source_frame.iter().enumerate() {
        if let Some(output_channels) = channel_mappings.get(source_channel) {
            // Process output channels in SIMD chunks
            let mut output_idx = 0;
            while output_idx + SIMD_WIDTH <= output_channels.len() {
                let indices = &output_channels[output_idx..output_idx + SIMD_WIDTH];
                
                // Load 8 indices and samples
                let mut indices_simd = [0i32; SIMD_WIDTH];
                let mut samples_simd = [0.0f32; SIMD_WIDTH];
                
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        indices_simd[i] = idx as i32;
                        samples_simd[i] = sample;
                    }
                }
                
                // Load current frame values
                let mut frame_values = [0.0f32; SIMD_WIDTH];
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        frame_values[i] = frame[idx];
                    }
                }
                
                // Perform SIMD addition
                let frame_vec = _mm256_loadu_ps(frame_values.as_ptr());
                let sample_vec = _mm256_loadu_ps(samples_simd.as_ptr());
                let result_vec = _mm256_add_ps(frame_vec, sample_vec);
                
                // Store results back
                let mut result = [0.0f32; SIMD_WIDTH];
                _mm256_storeu_ps(result.as_mut_ptr(), result_vec);
                
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        frame[idx] = result[i];
                    }
                }
                
                output_idx += SIMD_WIDTH;
            }
            
            // Handle remaining samples
            for &output_index in &output_channels[output_idx..] {
                if output_index < frame.len() {
                    frame[output_index] += sample;
                }
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn clear_buffer_avx2(buffer: &mut [f32]) {
    const SIMD_WIDTH: usize = 8;
    let mut idx = 0;
    
    // Process 8 samples at a time
    while idx + SIMD_WIDTH <= buffer.len() {
        let zero_vec = _mm256_setzero_ps();
        _mm256_storeu_ps(buffer.as_mut_ptr().add(idx), zero_vec);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..buffer.len() {
        buffer[i] = 0.0;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn copy_buffer_avx2(dst: &mut [f32], src: &[f32]) {
    const SIMD_WIDTH: usize = 8;
    let len = std::cmp::min(dst.len(), src.len());
    let mut idx = 0;
    
    // Process 8 samples at a time
    while idx + SIMD_WIDTH <= len {
        let src_vec = _mm256_loadu_ps(src.as_ptr().add(idx));
        _mm256_storeu_ps(dst.as_mut_ptr().add(idx), src_vec);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..len {
        dst[i] = src[i];
    }
}

// ============================================================================
// SSE4.1 Implementations
// ============================================================================

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn mix_samples_sse4(
    frame: &mut [f32],
    source_frame: &[f32],
    channel_mappings: &[Vec<usize>],
) {
    // Process 4 samples at a time with SSE4.1
    const SIMD_WIDTH: usize = 4;
    
    for (source_channel, &sample) in source_frame.iter().enumerate() {
        if let Some(output_channels) = channel_mappings.get(source_channel) {
            // Process output channels in SIMD chunks
            let mut output_idx = 0;
            while output_idx + SIMD_WIDTH <= output_channels.len() {
                let indices = &output_channels[output_idx..output_idx + SIMD_WIDTH];
                
                // Load 4 indices and samples
                let mut indices_simd = [0i32; SIMD_WIDTH];
                let mut samples_simd = [0.0f32; SIMD_WIDTH];
                
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        indices_simd[i] = idx as i32;
                        samples_simd[i] = sample;
                    }
                }
                
                // Load current frame values
                let mut frame_values = [0.0f32; SIMD_WIDTH];
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        frame_values[i] = frame[idx];
                    }
                }
                
                // Perform SIMD addition
                let frame_vec = _mm_loadu_ps(frame_values.as_ptr());
                let sample_vec = _mm_loadu_ps(samples_simd.as_ptr());
                let result_vec = _mm_add_ps(frame_vec, sample_vec);
                
                // Store results back
                let mut result = [0.0f32; SIMD_WIDTH];
                _mm_storeu_ps(result.as_mut_ptr(), result_vec);
                
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        frame[idx] = result[i];
                    }
                }
                
                output_idx += SIMD_WIDTH;
            }
            
            // Handle remaining samples
            for &output_index in &output_channels[output_idx..] {
                if output_index < frame.len() {
                    frame[output_index] += sample;
                }
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn clear_buffer_sse2(buffer: &mut [f32]) {
    const SIMD_WIDTH: usize = 4;
    let mut idx = 0;
    
    // Process 4 samples at a time
    while idx + SIMD_WIDTH <= buffer.len() {
        let zero_vec = _mm_setzero_ps();
        _mm_storeu_ps(buffer.as_mut_ptr().add(idx), zero_vec);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..buffer.len() {
        buffer[i] = 0.0;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn copy_buffer_sse2(dst: &mut [f32], src: &[f32]) {
    const SIMD_WIDTH: usize = 4;
    let len = std::cmp::min(dst.len(), src.len());
    let mut idx = 0;
    
    // Process 4 samples at a time
    while idx + SIMD_WIDTH <= len {
        let src_vec = _mm_loadu_ps(src.as_ptr().add(idx));
        _mm_storeu_ps(dst.as_mut_ptr().add(idx), src_vec);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..len {
        dst[i] = src[i];
    }
}

// ============================================================================
// ARM NEON Implementations
// ============================================================================

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn mix_samples_neon(
    frame: &mut [f32],
    source_frame: &[f32],
    channel_mappings: &[Vec<usize>],
) {
    // Process 8 samples at a time with NEON (2x 4-sample vectors)
    const SIMD_WIDTH: usize = 8;
    
    for (source_channel, &sample) in source_frame.iter().enumerate() {
        if let Some(output_channels) = channel_mappings.get(source_channel) {
            // Process output channels in SIMD chunks
            let mut output_idx = 0;
            while output_idx + SIMD_WIDTH <= output_channels.len() {
                let indices = &output_channels[output_idx..output_idx + SIMD_WIDTH];
                
                // Load 8 indices and samples
                let mut indices_simd = [0i32; SIMD_WIDTH];
                let mut samples_simd = [0.0f32; SIMD_WIDTH];
                
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        indices_simd[i] = idx as i32;
                        samples_simd[i] = sample;
                    }
                }
                
                // Load current frame values
                let mut frame_values = [0.0f32; SIMD_WIDTH];
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        frame_values[i] = frame[idx];
                    }
                }
                
                // Perform SIMD addition (2x 4-sample vectors)
                let frame_vec1 = vld1q_f32(frame_values.as_ptr());
                let sample_vec1 = vld1q_f32(samples_simd.as_ptr());
                let result_vec1 = vaddq_f32(frame_vec1, sample_vec1);
                
                let frame_vec2 = vld1q_f32(frame_values.as_ptr().add(4));
                let sample_vec2 = vld1q_f32(samples_simd.as_ptr().add(4));
                let result_vec2 = vaddq_f32(frame_vec2, sample_vec2);
                
                // Store results back
                let mut result = [0.0f32; SIMD_WIDTH];
                vst1q_f32(result.as_mut_ptr(), result_vec1);
                vst1q_f32(result.as_mut_ptr().add(4), result_vec2);
                
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        frame[idx] = result[i];
                    }
                }
                
                output_idx += SIMD_WIDTH;
            }
            
            // Handle remaining samples
            for &output_index in &output_channels[output_idx..] {
                if output_index < frame.len() {
                    frame[output_index] += sample;
                }
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn clear_buffer_neon(buffer: &mut [f32]) {
    const SIMD_WIDTH: usize = 8;  // Process 8 samples at a time
    let mut idx = 0;
    
    // Process 8 samples at a time (2x 4-sample vectors)
    while idx + SIMD_WIDTH <= buffer.len() {
        let zero_vec = vdupq_n_f32(0.0);
        vst1q_f32(buffer.as_mut_ptr().add(idx), zero_vec);
        vst1q_f32(buffer.as_mut_ptr().add(idx + 4), zero_vec);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..buffer.len() {
        buffer[i] = 0.0;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn copy_buffer_neon(dst: &mut [f32], src: &[f32]) {
    const SIMD_WIDTH: usize = 8;  // Process 8 samples at a time
    let len = std::cmp::min(dst.len(), src.len());
    let mut idx = 0;
    
    // Process 8 samples at a time (2x 4-sample vectors)
    while idx + SIMD_WIDTH <= len {
        let src_vec1 = vld1q_f32(src.as_ptr().add(idx));
        let src_vec2 = vld1q_f32(src.as_ptr().add(idx + 4));
        vst1q_f32(dst.as_mut_ptr().add(idx), src_vec1);
        vst1q_f32(dst.as_mut_ptr().add(idx + 4), src_vec2);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..len {
        dst[i] = src[i];
    }
}

// ============================================================================
// Scalar Fallback Implementations
// ============================================================================

fn mix_samples_scalar(
    frame: &mut [f32],
    source_frame: &[f32],
    channel_mappings: &[Vec<usize>],
) {
    for (source_channel, &sample) in source_frame.iter().enumerate() {
        if let Some(output_channels) = channel_mappings.get(source_channel) {
            for &output_index in output_channels {
                if output_index < frame.len() {
                    frame[output_index] += sample;
                }
            }
        }
    }
}

fn clear_buffer_scalar(buffer: &mut [f32]) {
    for sample in buffer.iter_mut() {
        *sample = 0.0;
    }
}

fn copy_buffer_scalar(dst: &mut [f32], src: &[f32]) {
    let len = std::cmp::min(dst.len(), src.len());
    for i in 0..len {
        dst[i] = src[i];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_mix_samples_equivalence() {
        let mut frame_simd = vec![0.0f32; 8];
        let mut frame_scalar = vec![0.0f32; 8];
        let source_frame = vec![1.0, 2.0, 3.0, 4.0];
        let channel_mappings = vec![
            vec![0, 1],      // source 0 -> outputs 0, 1
            vec![2, 3],      // source 1 -> outputs 2, 3
            vec![4, 5],      // source 2 -> outputs 4, 5
            vec![6, 7],      // source 3 -> outputs 6, 7
        ];
        
        // Test SIMD implementation
        mix_samples_simd(&mut frame_simd, &source_frame, &channel_mappings);
        
        // Test scalar implementation
        mix_samples_scalar(&mut frame_scalar, &source_frame, &channel_mappings);
        
        // Results should be identical
        for (simd, scalar) in frame_simd.iter().zip(frame_scalar.iter()) {
            assert!((simd - scalar).abs() < 1e-6, "SIMD and scalar results differ");
        }
    }
    
    #[test]
    fn test_clear_buffer_equivalence() {
        let mut buffer_simd = vec![1.0f32; 16];
        let mut buffer_scalar = vec![1.0f32; 16];
        
        clear_buffer_simd(&mut buffer_simd);
        clear_buffer_scalar(&mut buffer_scalar);
        
        for (simd, scalar) in buffer_simd.iter().zip(buffer_scalar.iter()) {
            assert!((simd - scalar).abs() < 1e-6, "SIMD and scalar results differ");
        }
    }
    
    #[test]
    fn test_copy_buffer_equivalence() {
        let src = vec![1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let mut dst_simd = vec![0.0f32; 8];
        let mut dst_scalar = vec![0.0f32; 8];
        
        copy_buffer_simd(&mut dst_simd, &src);
        copy_buffer_scalar(&mut dst_scalar, &src);
        
        for (simd, scalar) in dst_simd.iter().zip(dst_scalar.iter()) {
            assert!((simd - scalar).abs() < 1e-6, "SIMD and scalar results differ");
        }
    }
}
