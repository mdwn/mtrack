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
/// Optimized for high-throughput audio processing (hundreds of thousands of samples/second)
pub fn mix_samples_simd(
    frame: &mut [f32],
    source_frame: &[f32],
    channel_mappings: &[Vec<usize>],
) {
    // For high-throughput audio, use batch processing when possible
    // Only use batch processing for very large buffers to amortize SIMD overhead
    if frame.len() >= 64 && source_frame.len() >= 8 {
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx2") {
                unsafe {
                    mix_samples_avx2_batch(frame, source_frame, channel_mappings);
                    return;
                }
            }
            if is_x86_feature_detected!("sse4.1") {
                unsafe {
                    mix_samples_sse4_batch(frame, source_frame, channel_mappings);
                    return;
                }
            }
        }
        
        #[cfg(target_arch = "aarch64")]
        {
            if is_aarch64_feature_detected!("neon") {
                unsafe {
                    mix_samples_neon_batch(frame, source_frame, channel_mappings);
                    return;
                }
            }
        }
    }
    
    // Fallback to original implementations for smaller buffers
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


/// SIMD-optimized sample conversion from i32 to f32 with scaling
/// Converts integer samples to floating point with proper scaling
pub fn convert_samples_simd(dst: &mut [f32], src: &[i32], scale_factor: f32) {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe {
                convert_samples_avx2(dst, src, scale_factor);
                return;
            }
        }
        if is_x86_feature_detected!("sse4.1") {
            unsafe {
                convert_samples_sse4(dst, src, scale_factor);
                return;
            }
        }
    }
    
    #[cfg(target_arch = "aarch64")]
    {
        if is_aarch64_feature_detected!("neon") {
            unsafe {
                convert_samples_neon(dst, src, scale_factor);
                return;
            }
        }
    }
    
    // Fallback to scalar implementation
    convert_samples_scalar(dst, src, scale_factor);
}

/// SIMD-optimized maximum amplitude calculation
/// Finds the maximum absolute value in a buffer
pub fn max_amplitude_simd(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe {
                return max_amplitude_avx2(samples);
            }
        }
        if is_x86_feature_detected!("sse4.1") {
            unsafe {
                return max_amplitude_sse4(samples);
            }
        }
    }
    
    #[cfg(target_arch = "aarch64")]
    {
        if is_aarch64_feature_detected!("neon") {
            unsafe {
                return max_amplitude_neon(samples);
            }
        }
    }
    
    // Fallback to scalar implementation
    max_amplitude_scalar(samples)
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

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn calculate_rms_avx2(samples: &[f32]) -> f32 {
    const SIMD_WIDTH: usize = 8;
    let mut sum_squares = 0.0f32;
    let mut idx = 0;
    
    // Process 8 samples at a time
    while idx + SIMD_WIDTH <= samples.len() {
        let sample_vec = _mm256_loadu_ps(samples.as_ptr().add(idx));
        let squared_vec = _mm256_mul_ps(sample_vec, sample_vec);
        
        // Horizontal sum of squared values
        let sum_vec = _mm256_hadd_ps(squared_vec, squared_vec);
        let sum_vec2 = _mm256_hadd_ps(sum_vec, sum_vec);
        let sum_scalar = _mm256_cvtss_f32(_mm256_add_ps(sum_vec2, _mm256_permute2f128_ps(sum_vec2, sum_vec2, 0x11)));
        
        sum_squares += sum_scalar;
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..samples.len() {
        sum_squares += samples[i] * samples[i];
    }
    
    (sum_squares / samples.len() as f32).sqrt()
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn convert_samples_avx2(dst: &mut [f32], src: &[i32], scale_factor: f32) {
    const SIMD_WIDTH: usize = 8;
    let len = std::cmp::min(dst.len(), src.len());
    let mut idx = 0;
    
    // Broadcast scale factor to all elements
    let scale_vec = _mm256_set1_ps(scale_factor);
    
    // Process 8 samples at a time
    while idx + SIMD_WIDTH <= len {
        // Load i32 samples and convert to f32
        let src_vec = _mm256_loadu_si256(src.as_ptr().add(idx) as *const __m256i);
        let float_vec = _mm256_cvtepi32_ps(src_vec);
        let scaled_vec = _mm256_mul_ps(float_vec, scale_vec);
        
        _mm256_storeu_ps(dst.as_mut_ptr().add(idx), scaled_vec);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..len {
        dst[i] = src[i] as f32 * scale_factor;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn max_amplitude_avx2(samples: &[f32]) -> f32 {
    const SIMD_WIDTH: usize = 8;
    let mut max_abs = 0.0f32;
    let mut idx = 0;
    
    // Process 8 samples at a time
    while idx + SIMD_WIDTH <= samples.len() {
        let sample_vec = _mm256_loadu_ps(samples.as_ptr().add(idx));
        let abs_vec = _mm256_andnot_ps(_mm256_set1_ps(-0.0), sample_vec); // Absolute value
        
        // Find maximum in the vector using proper horizontal reduction
        let max_vec1 = _mm256_max_ps(abs_vec, _mm256_permute2f128_ps(abs_vec, abs_vec, 0x11));
        let max_vec2 = _mm256_max_ps(max_vec1, _mm256_permute_ps(max_vec1, 0x0E));
        let max_vec3 = _mm256_max_ps(max_vec2, _mm256_permute_ps(max_vec2, 0x11));
        let max_scalar = _mm256_cvtss_f32(max_vec3);
        
        max_abs = max_abs.max(max_scalar);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..samples.len() {
        max_abs = max_abs.max(samples[i].abs());
    }
    
    max_abs
}

// ============================================================================
// High-Throughput Batch Implementations (Optimized for Large Buffers)
// ============================================================================

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn mix_samples_avx2_batch(
    frame: &mut [f32],
    source_frame: &[f32],
    channel_mappings: &[Vec<usize>],
) {
    // Process 16 samples at a time with AVX2 for high throughput
    const SIMD_WIDTH: usize = 16;
    
    // Pre-allocate working arrays to avoid repeated allocations
    let mut frame_values = [0.0f32; SIMD_WIDTH];
    let mut samples_simd = [0.0f32; SIMD_WIDTH];
    let mut result = [0.0f32; SIMD_WIDTH];
    
    for (source_channel, &sample) in source_frame.iter().enumerate() {
        if let Some(output_channels) = channel_mappings.get(source_channel) {
            // Process output channels in larger SIMD chunks
            let mut output_idx = 0;
            while output_idx + SIMD_WIDTH <= output_channels.len() {
                let indices = &output_channels[output_idx..output_idx + SIMD_WIDTH];
                
                // Fill samples array with the same value (broadcast)
                samples_simd.fill(sample);
                
                // Load current frame values
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        frame_values[i] = frame[idx];
                    }
                }
                
                // Process 16 samples at once with AVX2
                let frame_vec1 = _mm256_loadu_ps(frame_values.as_ptr());
                let sample_vec1 = _mm256_loadu_ps(samples_simd.as_ptr());
                let result_vec1 = _mm256_add_ps(frame_vec1, sample_vec1);
                
                let frame_vec2 = _mm256_loadu_ps(frame_values.as_ptr().add(8));
                let sample_vec2 = _mm256_loadu_ps(samples_simd.as_ptr().add(8));
                let result_vec2 = _mm256_add_ps(frame_vec2, sample_vec2);
                
                // Store results back
                _mm256_storeu_ps(result.as_mut_ptr(), result_vec1);
                _mm256_storeu_ps(result.as_mut_ptr().add(8), result_vec2);
                
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
#[target_feature(enable = "sse4.1")]
unsafe fn mix_samples_sse4_batch(
    frame: &mut [f32],
    source_frame: &[f32],
    channel_mappings: &[Vec<usize>],
) {
    // Process 8 samples at a time with SSE4.1 for high throughput
    const SIMD_WIDTH: usize = 8;
    
    // Pre-allocate working arrays
    let mut frame_values = [0.0f32; SIMD_WIDTH];
    let mut samples_simd = [0.0f32; SIMD_WIDTH];
    let mut result = [0.0f32; SIMD_WIDTH];
    
    for (source_channel, &sample) in source_frame.iter().enumerate() {
        if let Some(output_channels) = channel_mappings.get(source_channel) {
            // Process output channels in larger SIMD chunks
            let mut output_idx = 0;
            while output_idx + SIMD_WIDTH <= output_channels.len() {
                let indices = &output_channels[output_idx..output_idx + SIMD_WIDTH];
                
                // Fill samples array with the same value (broadcast)
                samples_simd.fill(sample);
                
                // Load current frame values
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        frame_values[i] = frame[idx];
                    }
                }
                
                // Process 8 samples at once with SSE4.1
                let frame_vec1 = _mm_loadu_ps(frame_values.as_ptr());
                let sample_vec1 = _mm_loadu_ps(samples_simd.as_ptr());
                let result_vec1 = _mm_add_ps(frame_vec1, sample_vec1);
                
                let frame_vec2 = _mm_loadu_ps(frame_values.as_ptr().add(4));
                let sample_vec2 = _mm_loadu_ps(samples_simd.as_ptr().add(4));
                let result_vec2 = _mm_add_ps(frame_vec2, sample_vec2);
                
                // Store results back
                _mm_storeu_ps(result.as_mut_ptr(), result_vec1);
                _mm_storeu_ps(result.as_mut_ptr().add(4), result_vec2);
                
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
unsafe fn mix_samples_neon_batch(
    frame: &mut [f32],
    source_frame: &[f32],
    channel_mappings: &[Vec<usize>],
) {
    // Process 8 samples at a time with NEON for high throughput
    const SIMD_WIDTH: usize = 8;
    
    // Pre-allocate working arrays
    let mut frame_values = [0.0f32; SIMD_WIDTH];
    let mut samples_simd = [0.0f32; SIMD_WIDTH];
    let mut result = [0.0f32; SIMD_WIDTH];
    
    for (source_channel, &sample) in source_frame.iter().enumerate() {
        if let Some(output_channels) = channel_mappings.get(source_channel) {
            // Process output channels in larger SIMD chunks
            let mut output_idx = 0;
            while output_idx + SIMD_WIDTH <= output_channels.len() {
                let indices = &output_channels[output_idx..output_idx + SIMD_WIDTH];
                
                // Fill samples array with the same value (broadcast)
                samples_simd.fill(sample);
                
                // Load current frame values
                for (i, &idx) in indices.iter().enumerate() {
                    if idx < frame.len() {
                        frame_values[i] = frame[idx];
                    }
                }
                
                // Process 8 samples at once with NEON
                let frame_vec1 = vld1q_f32(frame_values.as_ptr());
                let sample_vec1 = vld1q_f32(samples_simd.as_ptr());
                let result_vec1 = vaddq_f32(frame_vec1, sample_vec1);
                
                let frame_vec2 = vld1q_f32(frame_values.as_ptr().add(4));
                let sample_vec2 = vld1q_f32(samples_simd.as_ptr().add(4));
                let result_vec2 = vaddq_f32(frame_vec2, sample_vec2);
                
                // Store results back
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

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn calculate_rms_sse4(samples: &[f32]) -> f32 {
    const SIMD_WIDTH: usize = 4;
    let mut sum_squares = 0.0f32;
    let mut idx = 0;
    
    // Process 4 samples at a time
    while idx + SIMD_WIDTH <= samples.len() {
        let sample_vec = _mm_loadu_ps(samples.as_ptr().add(idx));
        let squared_vec = _mm_mul_ps(sample_vec, sample_vec);
        
        // Horizontal sum of squared values
        let sum_vec = _mm_hadd_ps(squared_vec, squared_vec);
        let sum_scalar = _mm_cvtss_f32(_mm_hadd_ps(sum_vec, sum_vec));
        
        sum_squares += sum_scalar;
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..samples.len() {
        sum_squares += samples[i] * samples[i];
    }
    
    (sum_squares / samples.len() as f32).sqrt()
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn convert_samples_sse4(dst: &mut [f32], src: &[i32], scale_factor: f32) {
    const SIMD_WIDTH: usize = 4;
    let len = std::cmp::min(dst.len(), src.len());
    let mut idx = 0;
    
    // Broadcast scale factor to all elements
    let scale_vec = _mm_set1_ps(scale_factor);
    
    // Process 4 samples at a time
    while idx + SIMD_WIDTH <= len {
        // Load i32 samples and convert to f32
        let src_vec = _mm_loadu_si128(src.as_ptr().add(idx) as *const __m128i);
        let float_vec = _mm_cvtepi32_ps(src_vec);
        let scaled_vec = _mm_mul_ps(float_vec, scale_vec);
        
        _mm_storeu_ps(dst.as_mut_ptr().add(idx), scaled_vec);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..len {
        dst[i] = src[i] as f32 * scale_factor;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn max_amplitude_sse4(samples: &[f32]) -> f32 {
    const SIMD_WIDTH: usize = 4;
    let mut max_abs = 0.0f32;
    let mut idx = 0;
    
    // Process 4 samples at a time
    while idx + SIMD_WIDTH <= samples.len() {
        let sample_vec = _mm_loadu_ps(samples.as_ptr().add(idx));
        let abs_vec = _mm_andnot_ps(_mm_set1_ps(-0.0), sample_vec); // Absolute value
        
        // Find maximum in the vector using proper horizontal reduction
        let max_vec1 = _mm_max_ps(abs_vec, _mm_permute_ps(abs_vec, 0x0E));
        let max_vec2 = _mm_max_ps(max_vec1, _mm_permute_ps(max_vec1, 0x11));
        let max_scalar = _mm_cvtss_f32(max_vec2);
        
        max_abs = max_abs.max(max_scalar);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..samples.len() {
        max_abs = max_abs.max(samples[i].abs());
    }
    
    max_abs
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
                
                // Load 8 indices and samples (2x 4-sample vectors)
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
                let frame_vec = vld1q_f32(frame_values.as_ptr());
                let sample_vec = vld1q_f32(samples_simd.as_ptr());
                let result_vec = vaddq_f32(frame_vec, sample_vec);
                
                // Store results back
                let mut result = [0.0f32; SIMD_WIDTH];
                vst1q_f32(result.as_mut_ptr(), result_vec);
                
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
    const SIMD_WIDTH: usize = 8;  // Process 8 samples at a time like SSE batch
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
    const SIMD_WIDTH: usize = 8;  // Process 8 samples at a time like SSE batch
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

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn calculate_rms_neon(samples: &[f32]) -> f32 {
    const SIMD_WIDTH: usize = 8;  // Process 8 samples at a time like SSE batch
    let mut sum_squares = 0.0f32;
    let mut idx = 0;
    
    // Process 8 samples at a time (2x 4-sample vectors)
    while idx + SIMD_WIDTH <= samples.len() {
        let sample_vec1 = vld1q_f32(samples.as_ptr().add(idx));
        let sample_vec2 = vld1q_f32(samples.as_ptr().add(idx + 4));
        
        let squared_vec1 = vmulq_f32(sample_vec1, sample_vec1);
        let squared_vec2 = vmulq_f32(sample_vec2, sample_vec2);
        
        // Horizontal sum of squared values for both vectors
        let sum_vec1 = vpaddq_f32(squared_vec1, squared_vec1);
        let sum_vec2 = vpaddq_f32(squared_vec2, squared_vec2);
        
        let sum1 = vgetq_lane_f32(sum_vec1, 0) + vgetq_lane_f32(sum_vec1, 1);
        let sum2 = vgetq_lane_f32(sum_vec2, 0) + vgetq_lane_f32(sum_vec2, 1);
        
        sum_squares += sum1 + sum2;
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..samples.len() {
        sum_squares += samples[i] * samples[i];
    }
    
    (sum_squares / samples.len() as f32).sqrt()
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn convert_samples_neon(dst: &mut [f32], src: &[i32], scale_factor: f32) {
    const SIMD_WIDTH: usize = 8;  // Process 8 samples at a time like SSE batch
    let len = std::cmp::min(dst.len(), src.len());
    let mut idx = 0;
    
    // Broadcast scale factor to all elements
    let scale_vec = vdupq_n_f32(scale_factor);
    
    // Process 8 samples at a time (2x 4-sample vectors)
    while idx + SIMD_WIDTH <= len {
        // Load i32 samples and convert to f32
        let src_vec1 = vld1q_s32(src.as_ptr().add(idx));
        let src_vec2 = vld1q_s32(src.as_ptr().add(idx + 4));
        
        let float_vec1 = vcvtq_f32_s32(src_vec1);
        let float_vec2 = vcvtq_f32_s32(src_vec2);
        
        let scaled_vec1 = vmulq_f32(float_vec1, scale_vec);
        let scaled_vec2 = vmulq_f32(float_vec2, scale_vec);
        
        vst1q_f32(dst.as_mut_ptr().add(idx), scaled_vec1);
        vst1q_f32(dst.as_mut_ptr().add(idx + 4), scaled_vec2);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..len {
        dst[i] = src[i] as f32 * scale_factor;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn max_amplitude_neon(samples: &[f32]) -> f32 {
    const SIMD_WIDTH: usize = 8;  // Process 8 samples at a time like SSE batch
    let mut max_abs = 0.0f32;
    let mut idx = 0;
    
    // Process 8 samples at a time (2x 4-sample vectors)
    while idx + SIMD_WIDTH <= samples.len() {
        let sample_vec1 = vld1q_f32(samples.as_ptr().add(idx));
        let sample_vec2 = vld1q_f32(samples.as_ptr().add(idx + 4));
        
        let abs_vec1 = vabsq_f32(sample_vec1); // Absolute value
        let abs_vec2 = vabsq_f32(sample_vec2);
        
        // Find maximum in both vectors
        let max_vec1 = vpmaxq_f32(abs_vec1, abs_vec1);
        let max_vec2 = vpmaxq_f32(abs_vec2, abs_vec2);
        
        let max_scalar1 = vgetq_lane_f32(max_vec1, 0).max(vgetq_lane_f32(max_vec1, 1));
        let max_scalar2 = vgetq_lane_f32(max_vec2, 0).max(vgetq_lane_f32(max_vec2, 1));
        
        max_abs = max_abs.max(max_scalar1).max(max_scalar2);
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..samples.len() {
        max_abs = max_abs.max(samples[i].abs());
    }
    
    max_abs
}

// ============================================================================
// High-Frequency Energy Calculation Implementations
// ============================================================================

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn calculate_high_frequency_energy_avx2(samples: &[f32]) -> f32 {
    const SIMD_WIDTH: usize = 8;
    let mut sum_squares = 0.0f32;
    let mut idx = 1; // Start from 1 since we need previous sample
    
    // Process 8 differences at a time
    while idx + SIMD_WIDTH <= samples.len() {
        let current_vec = _mm256_loadu_ps(samples.as_ptr().add(idx));
        let previous_vec = _mm256_loadu_ps(samples.as_ptr().add(idx - 1));
        let diff_vec = _mm256_sub_ps(current_vec, previous_vec);
        let squared_vec = _mm256_mul_ps(diff_vec, diff_vec);
        
        // Horizontal sum of squared differences
        let sum_vec = _mm256_hadd_ps(squared_vec, squared_vec);
        let sum_vec2 = _mm256_hadd_ps(sum_vec, sum_vec);
        let sum_scalar = _mm256_cvtss_f32(sum_vec2) + _mm256_cvtss_f32(_mm256_permutevar8x32_ps(sum_vec2, _mm256_set_epi32(0, 0, 0, 0, 0, 0, 0, 1)));
        
        sum_squares += sum_scalar;
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..samples.len() {
        let diff = samples[i] - samples[i - 1];
        sum_squares += diff * diff;
    }
    
    sum_squares / (samples.len() - 1) as f32
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn calculate_high_frequency_energy_sse4(samples: &[f32]) -> f32 {
    const SIMD_WIDTH: usize = 4;
    let mut sum_squares = 0.0f32;
    let mut idx = 1; // Start from 1 since we need previous sample
    
    // Process 4 differences at a time
    while idx + SIMD_WIDTH <= samples.len() {
        let current_vec = _mm_loadu_ps(samples.as_ptr().add(idx));
        let previous_vec = _mm_loadu_ps(samples.as_ptr().add(idx - 1));
        let diff_vec = _mm_sub_ps(current_vec, previous_vec);
        let squared_vec = _mm_mul_ps(diff_vec, diff_vec);
        
        // Horizontal sum of squared differences
        let sum_vec = _mm_hadd_ps(squared_vec, squared_vec);
        let sum_vec2 = _mm_hadd_ps(sum_vec, sum_vec);
        let sum_scalar = _mm_cvtss_f32(sum_vec2);
        
        sum_squares += sum_scalar;
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..samples.len() {
        let diff = samples[i] - samples[i - 1];
        sum_squares += diff * diff;
    }
    
    sum_squares / (samples.len() - 1) as f32
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn calculate_high_frequency_energy_neon(samples: &[f32]) -> f32 {
    const SIMD_WIDTH: usize = 8;  // Process 8 differences at a time
    let mut sum_squares = 0.0f32;
    let mut idx = 1; // Start from 1 since we need previous sample
    
    // Process 8 differences at a time (2x 4-sample vectors)
    while idx + SIMD_WIDTH <= samples.len() {
        let current_vec1 = vld1q_f32(samples.as_ptr().add(idx));
        let current_vec2 = vld1q_f32(samples.as_ptr().add(idx + 4));
        let previous_vec1 = vld1q_f32(samples.as_ptr().add(idx - 1));
        let previous_vec2 = vld1q_f32(samples.as_ptr().add(idx + 3));
        
        let diff_vec1 = vsubq_f32(current_vec1, previous_vec1);
        let diff_vec2 = vsubq_f32(current_vec2, previous_vec2);
        
        let squared_vec1 = vmulq_f32(diff_vec1, diff_vec1);
        let squared_vec2 = vmulq_f32(diff_vec2, diff_vec2);
        
        // Horizontal sum of squared differences for both vectors
        let sum_vec1 = vpaddq_f32(squared_vec1, squared_vec1);
        let sum_vec2 = vpaddq_f32(squared_vec2, squared_vec2);
        
        let sum1 = vgetq_lane_f32(sum_vec1, 0) + vgetq_lane_f32(sum_vec1, 1);
        let sum2 = vgetq_lane_f32(sum_vec2, 0) + vgetq_lane_f32(sum_vec2, 1);
        
        sum_squares += sum1 + sum2;
        idx += SIMD_WIDTH;
    }
    
    // Handle remaining samples
    for i in idx..samples.len() {
        let diff = samples[i] - samples[i - 1];
        sum_squares += diff * diff;
    }
    
    sum_squares / (samples.len() - 1) as f32
}

fn calculate_high_frequency_energy_scalar(samples: &[f32]) -> f32 {
    if samples.len() < 2 {
        return 0.0;
    }

    let mut high_freq_energy = 0.0;
    for i in 1..samples.len() {
        let diff = samples[i] - samples[i - 1];
        high_freq_energy += diff * diff;
    }

    high_freq_energy / (samples.len() - 1) as f32
}

// ============================================================================
// High-Level SIMD Functions with Runtime Feature Detection
// ============================================================================

/// Calculate RMS (Root Mean Square) of a signal using SIMD optimization
pub fn calculate_rms_simd(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            return unsafe { calculate_rms_avx2(samples) };
        }
        if is_x86_feature_detected!("sse4.1") {
            return unsafe { calculate_rms_sse4(samples) };
        }
    }
    
    #[cfg(target_arch = "aarch64")]
    {
        if is_aarch64_feature_detected!("neon") {
            return unsafe { calculate_rms_neon(samples) };
        }
    }
    
    // Fallback to scalar implementation
    calculate_rms_scalar(samples)
}

/// Calculate high-frequency energy content using SIMD optimization
pub fn calculate_high_frequency_energy_simd(samples: &[f32]) -> f32 {
    if samples.len() < 2 {
        return 0.0;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            return unsafe { calculate_high_frequency_energy_avx2(samples) };
        }
        if is_x86_feature_detected!("sse4.1") {
            return unsafe { calculate_high_frequency_energy_sse4(samples) };
        }
    }
    
    #[cfg(target_arch = "aarch64")]
    {
        if is_aarch64_feature_detected!("neon") {
            return unsafe { calculate_high_frequency_energy_neon(samples) };
        }
    }
    
    // Fallback to scalar implementation
    calculate_high_frequency_energy_scalar(samples)
}

/// Create a vector initialized with zeros using SIMD optimization
pub fn create_zero_vector(len: usize) -> Vec<f32> {
    let mut vec = vec![0.0f32; len];
    clear_buffer_simd(&mut vec);
    vec
}

/// Fill a vector with a specific value using SIMD optimization
pub fn fill_vector_simd(vec: &mut [f32], value: f32) {
    if value == 0.0 {
        clear_buffer_simd(vec);
    } else {
        // For non-zero values, we need a scalar implementation
        // This could be optimized further with SIMD broadcasting
        for sample in vec.iter_mut() {
            *sample = value;
        }
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

fn calculate_rms_scalar(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    
    let sum_squares: f32 = samples.iter().map(|&x| x * x).sum();
    (sum_squares / samples.len() as f32).sqrt()
}

fn convert_samples_scalar(dst: &mut [f32], src: &[i32], scale_factor: f32) {
    let len = std::cmp::min(dst.len(), src.len());
    for i in 0..len {
        dst[i] = src[i] as f32 * scale_factor;
    }
}

fn max_amplitude_scalar(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    
    samples.iter().map(|&x| x.abs()).fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    
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
    
    #[test]
    fn test_simd_performance() {
        // Create large test data for performance comparison
        const BUFFER_SIZE: usize = 1024 * 16; // 16KB buffer
        const ITERATIONS: usize = 1000;
        
        let mut buffer_simd = vec![1.0f32; BUFFER_SIZE];
        let mut buffer_scalar = vec![1.0f32; BUFFER_SIZE];
        
        // Benchmark SIMD clear
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            clear_buffer_simd(&mut buffer_simd);
        }
        let simd_time = start.elapsed();
        
        // Benchmark scalar clear
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            clear_buffer_scalar(&mut buffer_scalar);
        }
        let scalar_time = start.elapsed();
        
        println!("SIMD clear time: {:?}", simd_time);
        println!("Scalar clear time: {:?}", scalar_time);
        
        // SIMD should be faster (or at least not slower)
        // We don't assert this as it depends on the platform
        println!("SIMD speedup: {:.2}x", scalar_time.as_nanos() as f64 / simd_time.as_nanos() as f64);
    }
    
    #[test]
    fn test_mix_performance() {
        // Create realistic audio mixing scenario
        const NUM_CHANNELS: usize = 32;
        const NUM_SOURCES: usize = 8;
        const ITERATIONS: usize = 100;
        
        let mut frame_simd = vec![0.0f32; NUM_CHANNELS];
        let mut frame_scalar = vec![0.0f32; NUM_CHANNELS];
        
        // Create test data
        let source_frames: Vec<Vec<f32>> = (0..NUM_SOURCES)
            .map(|i| (0..4).map(|j| (i as f32 + j as f32) * 0.1).collect())
            .collect();
        
        let channel_mappings: Vec<Vec<usize>> = (0..NUM_SOURCES)
            .map(|i| vec![i * 4, i * 4 + 1, i * 4 + 2, i * 4 + 3])
            .collect();
        
        // Benchmark SIMD mixing
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            frame_simd.fill(0.0);
            for (source_frame, mappings) in source_frames.iter().zip(channel_mappings.iter()) {
                mix_samples_simd(&mut frame_simd, source_frame, &[mappings.clone()]);
            }
        }
        let simd_time = start.elapsed();
        
        // Benchmark scalar mixing
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            frame_scalar.fill(0.0);
            for (source_frame, mappings) in source_frames.iter().zip(channel_mappings.iter()) {
                mix_samples_scalar(&mut frame_scalar, source_frame, &[mappings.clone()]);
            }
        }
        let scalar_time = start.elapsed();
        
        println!("SIMD mix time: {:?}", simd_time);
        println!("Scalar mix time: {:?}", scalar_time);
        println!("SIMD speedup: {:.2}x", scalar_time.as_nanos() as f64 / simd_time.as_nanos() as f64);
        
        // Verify results are equivalent
        for (simd, scalar) in frame_simd.iter().zip(frame_scalar.iter()) {
            assert!((simd - scalar).abs() < 1e-6, "SIMD and scalar results differ");
        }
    }
    
    #[test]
    fn test_arm_neon_equivalence() {
        // Test ARM NEON implementations if available
        #[cfg(target_arch = "aarch64")]
        {
            if is_aarch64_feature_detected!("neon") {
                let mut frame_simd = vec![0.0f32; 8];
                let mut frame_scalar = vec![0.0f32; 8];
                let source_frame = vec![1.0, 2.0, 3.0, 4.0];
                let channel_mappings = vec![
                    vec![0, 1],      // source 0 -> outputs 0, 1
                    vec![2, 3],      // source 1 -> outputs 2, 3
                    vec![4, 5],      // source 2 -> outputs 4, 5
                    vec![6, 7],      // source 3 -> outputs 6, 7
                ];
                
                // Test NEON implementation
                unsafe {
                    mix_samples_neon(&mut frame_simd, &source_frame, &channel_mappings);
                }
                
                // Test scalar implementation
                mix_samples_scalar(&mut frame_scalar, &source_frame, &channel_mappings);
                
                // Results should be identical
                for (neon, scalar) in frame_simd.iter().zip(frame_scalar.iter()) {
                    assert!((neon - scalar).abs() < 1e-6, "NEON and scalar results differ");
                }
                
                println!("ARM NEON implementation verified");
            } else {
                println!("ARM NEON not available on this platform");
            }
        }
        
        #[cfg(not(target_arch = "aarch64"))]
        {
            println!("ARM NEON test skipped (not on aarch64)");
        }
    }
    
    #[test]
    fn test_high_throughput_performance() {
        // Test realistic high-throughput scenario: 64+ channels, 16+ sources, large buffers
        const NUM_CHANNELS: usize = 128;  // Large channel count
        const NUM_SOURCES: usize = 16;    // Many sources
        const ITERATIONS: usize = 100;    // Fewer iterations for large dataset
        
        let mut frame_simd = vec![0.0f32; NUM_CHANNELS];
        let mut frame_scalar = vec![0.0f32; NUM_CHANNELS];
        
        // Create test data for realistic high-throughput scenario
        let source_frames: Vec<Vec<f32>> = (0..NUM_SOURCES)
            .map(|i| (0..8).map(|j| (i as f32 + j as f32) * 0.1).collect())  // 8 samples per source
            .collect();
        
        let channel_mappings: Vec<Vec<usize>> = (0..NUM_SOURCES)
            .map(|i| vec![i * 8, i * 8 + 1, i * 8 + 2, i * 8 + 3, i * 8 + 4, i * 8 + 5, i * 8 + 6, i * 8 + 7])
            .collect();
        
        // Benchmark high-throughput SIMD mixing
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            frame_simd.fill(0.0);
            for (source_frame, mappings) in source_frames.iter().zip(channel_mappings.iter()) {
                mix_samples_simd(&mut frame_simd, source_frame, &[mappings.clone()]);
            }
        }
        let simd_time = start.elapsed();
        
        // Benchmark scalar mixing
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            frame_scalar.fill(0.0);
            for (source_frame, mappings) in source_frames.iter().zip(channel_mappings.iter()) {
                mix_samples_scalar(&mut frame_scalar, source_frame, &[mappings.clone()]);
            }
        }
        let scalar_time = start.elapsed();
        
        println!("High-throughput SIMD time: {:?}", simd_time);
        println!("High-throughput Scalar time: {:?}", scalar_time);
        println!("High-throughput SIMD speedup: {:.2}x", scalar_time.as_nanos() as f64 / simd_time.as_nanos() as f64);
        
        // Calculate samples per second
        let samples_per_iteration = NUM_CHANNELS * NUM_SOURCES * 8; // 8 samples per source
        let total_samples = samples_per_iteration * ITERATIONS;
        let simd_samples_per_sec = total_samples as f64 / simd_time.as_secs_f64();
        let scalar_samples_per_sec = total_samples as f64 / scalar_time.as_secs_f64();
        
        println!("SIMD throughput: {:.0} samples/sec", simd_samples_per_sec);
        println!("Scalar throughput: {:.0} samples/sec", scalar_samples_per_sec);
        
        // Verify results are equivalent
        for (simd, scalar) in frame_simd.iter().zip(frame_scalar.iter()) {
            assert!((simd - scalar).abs() < 1e-6, "SIMD and scalar results differ");
        }
    }
    
    #[test]
    fn test_buffer_operations_performance() {
        // Test buffer operations which should show clear SIMD benefits
        const BUFFER_SIZE: usize = 1024 * 16; // 16KB buffer
        const ITERATIONS: usize = 1000;
        
        let mut buffer_simd = vec![1.0f32; BUFFER_SIZE];
        let mut buffer_scalar = vec![1.0f32; BUFFER_SIZE];
        let src_buffer = vec![0.5f32; BUFFER_SIZE];
        
        // Benchmark SIMD buffer operations
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            clear_buffer_simd(&mut buffer_simd);
            copy_buffer_simd(&mut buffer_simd, &src_buffer);
        }
        let simd_time = start.elapsed();
        
        // Benchmark scalar buffer operations
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            clear_buffer_scalar(&mut buffer_scalar);
            copy_buffer_scalar(&mut buffer_scalar, &src_buffer);
        }
        let scalar_time = start.elapsed();
        
        println!("Buffer operations SIMD time: {:?}", simd_time);
        println!("Buffer operations Scalar time: {:?}", scalar_time);
        println!("Buffer operations SIMD speedup: {:.2}x", scalar_time.as_nanos() as f64 / simd_time.as_nanos() as f64);
        
        // Calculate throughput
        let bytes_per_iteration = BUFFER_SIZE * 4; // 4 bytes per f32
        let total_bytes = bytes_per_iteration * ITERATIONS * 2; // 2 operations per iteration
        let simd_throughput = total_bytes as f64 / simd_time.as_secs_f64() / 1_000_000.0; // MB/s
        let scalar_throughput = total_bytes as f64 / scalar_time.as_secs_f64() / 1_000_000.0; // MB/s
        
        println!("SIMD throughput: {:.1} MB/s", simd_throughput);
        println!("Scalar throughput: {:.1} MB/s", scalar_throughput);
    }
    
    #[test]
    fn test_rms_equivalence() {
        let samples = vec![1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        
        let rms_simd = calculate_rms_simd(&samples);
        let rms_scalar = calculate_rms_scalar(&samples);
        
        assert!((rms_simd - rms_scalar).abs() < 1e-6, "RMS SIMD and scalar results differ");
    }
    
    #[test]
    fn test_sample_conversion_equivalence() {
        let src = vec![1000i32, -2000, 3000, -4000, 5000];
        let scale_factor = 0.001f32;
        let mut dst_simd = vec![0.0f32; 5];
        let mut dst_scalar = vec![0.0f32; 5];
        
        convert_samples_simd(&mut dst_simd, &src, scale_factor);
        convert_samples_scalar(&mut dst_scalar, &src, scale_factor);
        
        for (simd, scalar) in dst_simd.iter().zip(dst_scalar.iter()) {
            assert!((simd - scalar).abs() < 1e-6, "Sample conversion SIMD and scalar results differ");
        }
    }
    
    #[test]
    fn test_max_amplitude_equivalence() {
        let samples = vec![1.0f32, -2.0, 3.0, -4.0, 5.0, -6.0, 7.0, -8.0];
        
        let max_simd = max_amplitude_simd(&samples);
        let max_scalar = max_amplitude_scalar(&samples);
        
        println!("SIMD max: {}, Scalar max: {}", max_simd, max_scalar);
        assert!((max_simd - max_scalar).abs() < 1e-6, "Max amplitude SIMD and scalar results differ");
    }
    
    #[test]
    fn test_sample_source_performance() {
        // Test sample source operations that would benefit from SIMD
        const BUFFER_SIZE: usize = 1024 * 8; // 8KB buffer
        const ITERATIONS: usize = 1000;
        
        let samples = (0..BUFFER_SIZE).map(|i| (i as f32 * 0.01).sin()).collect::<Vec<f32>>();
        let src_samples = (0..BUFFER_SIZE).map(|i| (i as i32) % 1000).collect::<Vec<i32>>();
        let mut dst_samples = vec![0.0f32; BUFFER_SIZE];
        let scale_factor = 0.001f32;
        
        // Benchmark RMS calculation
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            let _rms = calculate_rms_simd(&samples);
        }
        let rms_simd_time = start.elapsed();
        
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            let _rms = calculate_rms_scalar(&samples);
        }
        let rms_scalar_time = start.elapsed();
        
        // Benchmark sample conversion
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            convert_samples_simd(&mut dst_samples, &src_samples, scale_factor);
        }
        let convert_simd_time = start.elapsed();
        
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            convert_samples_scalar(&mut dst_samples, &src_samples, scale_factor);
        }
        let convert_scalar_time = start.elapsed();
        
        // Benchmark max amplitude
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            let _max = max_amplitude_simd(&samples);
        }
        let max_simd_time = start.elapsed();
        
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            let _max = max_amplitude_scalar(&samples);
        }
        let max_scalar_time = start.elapsed();
        
        println!("RMS SIMD time: {:?}", rms_simd_time);
        println!("RMS Scalar time: {:?}", rms_scalar_time);
        println!("RMS SIMD speedup: {:.2}x", rms_scalar_time.as_nanos() as f64 / rms_simd_time.as_nanos() as f64);
        
        println!("Convert SIMD time: {:?}", convert_simd_time);
        println!("Convert Scalar time: {:?}", convert_scalar_time);
        println!("Convert SIMD speedup: {:.2}x", convert_scalar_time.as_nanos() as f64 / convert_simd_time.as_nanos() as f64);
        
        println!("Max amplitude SIMD time: {:?}", max_simd_time);
        println!("Max amplitude Scalar time: {:?}", max_scalar_time);
        println!("Max amplitude SIMD speedup: {:.2}x", max_scalar_time.as_nanos() as f64 / max_simd_time.as_nanos() as f64);
    }
}
