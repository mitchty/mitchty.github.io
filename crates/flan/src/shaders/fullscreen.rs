//! Fullscreen post-process effect shaders.
//!
//! Each effect has two modules for now.
//!
//! * `{name}` Bevy runtime variant.
//!   `@group(0) @binding(0)` -> `screen_texture: texture_2d<f32>`
//!   `@group(0) @binding(1)` -> `texture_sampler: sampler`
//!   `@group(0) @binding(2)` -> `settings: FullscreenEffectSettings` (uniform,
//!   dynamic offset)
//!
//! * `{name}_test` headless wgpu test variant. Just the
//!   settings uniform at `@group(0) @binding(0)` and a dum intensity to grey
//!   stub body so the test harness can compile and render it.
// TODO: My minds a pumpkin but I stripped out the wgpu test crap, too sick of
// reviewing old comments I shouldn't have added in teh first place as comments
// are always out of date especially in the midst of fever dreams.
use wgsl_rs::wgsl;

#[cfg(not(all(feature = "webgl", target_arch = "wasm32")))]
#[wgsl]
pub mod fullscreen_settings {
    use wgsl_rs::std::*;

    #[derive(Wgsl)]
    pub struct GlobalsUniform {
        pub time: f32,
        pub delta_time: f32,
        pub frame_count: u32,
    }

    #[derive(Wgsl)]
    pub struct FullscreenEffectSettings {
        pub intensity: f32,
        pub _pad0: f32,
        pub _pad1: f32,
        pub _pad2: f32,
    }
}

#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
#[wgsl]
pub mod fullscreen_settings {
    use wgsl_rs::std::*;

    #[derive(Wgsl)]
    pub struct GlobalsUniform {
        pub time: f32,
        pub delta_time: f32,
        pub frame_count: u32,
        pub frame_count_wrapped: u32,
    }

    /// 16-byte settings struct.
    #[derive(Wgsl)]
    pub struct FullscreenEffectSettings {
        pub intensity: f32,
        pub _pad0: f32,
        pub _pad1: f32,
        pub _pad2: f32,
    }
}

/// Bevy runtime variant
#[wgsl]
pub mod chromatic_aberration {
    use super::fullscreen_settings::*;
    use wgsl_rs::std::*;

    texture!(group(0), binding(0), SCREEN_TEXTURE: Texture2D<f32>);
    sampler!(group(0), binding(1), TEXTURE_SAMPLER: Sampler);
    uniform!(group(0), binding(2), SETTINGS: FullscreenEffectSettings);
    uniform!(group(1), binding(0), GLOBALS: GlobalsUniform);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        let offset_strength = get!(SETTINGS).intensity * 0.005;
        let r = texture_sample(
            SCREEN_TEXTURE,
            TEXTURE_SAMPLER,
            input.uv + vec2f(offset_strength, -offset_strength),
        )
        .x();
        let g = texture_sample(
            SCREEN_TEXTURE,
            TEXTURE_SAMPLER,
            input.uv + vec2f(-offset_strength, 0.0),
        )
        .y();
        let b = texture_sample(
            SCREEN_TEXTURE,
            TEXTURE_SAMPLER,
            input.uv + vec2f(0.0, offset_strength),
        )
        .z();
        vec4f(r, g, b, 1.0)
    }
}

/// Headless test variant.
#[wgsl]
pub mod chromatic_aberration_test {
    use super::fullscreen_settings::*;
    use wgsl_rs::std::*;

    uniform!(group(0), binding(0), SETTINGS: FullscreenEffectSettings);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    #[fragment]
    pub fn fragment(_input: FragmentInput) -> Vec4f {
        let v = clamp(get!(SETTINGS).intensity / 10.0, 0.0, 1.0);
        vec4f(v, v, v, 1.0)
    }
}

/// Math helpers for the VHS effect shader.
#[wgsl]
pub mod vhs_helpers {
    use wgsl_rs::std::*;

    pub fn rand(co: Vec2f) -> f32 {
        fract(sin(dot(co, vec2f(12.9898, 78.233))) * 43758.547)
    }

    pub fn vertical_bar(pos: f32, uv_y: f32, offset: f32) -> f32 {
        let range: f32 = 0.05;
        let edge0 = pos - range;
        let edge1 = pos + range;
        let mut x = smoothstep(edge0, pos, uv_y) * offset;
        x -= smoothstep(pos, edge1, uv_y) * offset;
        x
    }
}

#[wgsl]
pub mod vhs_effect {
    use super::fullscreen_settings::*;
    use super::vhs_helpers::*;
    use wgsl_rs::std::*;

    texture!(group(0), binding(0), SCREEN_TEXTURE: Texture2D<f32>);
    sampler!(group(0), binding(1), TEXTURE_SAMPLER: Sampler);
    uniform!(group(0), binding(2), SETTINGS: FullscreenEffectSettings);
    uniform!(group(1), binding(0), GLOBALS: GlobalsUniform);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        let noise_quality: f32 = 250.0;
        let noise_intensity: f32 = 0.0088;
        let offset_intensity: f32 = 0.02;
        let color_offset: f32 = 1.3;

        let intensity = get!(SETTINGS).intensity;
        let t = get!(GLOBALS).time % 6000.0;

        let mut uv = input.uv;

        let mut i: f32 = 0.0;
        while i < 0.71 {
            let d = (t * i) % 1.7;
            let mut o = sin(1.0 - tan(t * 0.24 * i));
            o *= offset_intensity * intensity;
            uv = vec2f(uv.x() + vertical_bar(d, uv.y(), o), uv.y());
            i += 0.1313;
        }

        let mut uv_y = uv.y();
        uv_y *= noise_quality;
        uv_y = (uv_y as i32) as f32 * (1.0 / noise_quality);
        let noise = rand(vec2f(t * 0.00001, uv_y));
        uv = vec2f(uv.x() + noise * noise_intensity * intensity, uv.y());

        let offset_r = vec2f(0.006 * sin(t), 0.0) * color_offset * intensity;
        let offset_g = vec2f(0.0073 * cos(t * 0.97), 0.0) * color_offset * intensity;

        let r = texture_sample(SCREEN_TEXTURE, TEXTURE_SAMPLER, uv + offset_r).x();
        let g = texture_sample(SCREEN_TEXTURE, TEXTURE_SAMPLER, uv + offset_g).y();
        let b = texture_sample(SCREEN_TEXTURE, TEXTURE_SAMPLER, uv).z();

        vec4f(r, g, b, 1.0)
    }
}

#[wgsl]
pub mod vhs_effect_test {
    use super::fullscreen_settings::*;
    use wgsl_rs::std::*;

    uniform!(group(0), binding(0), SETTINGS: FullscreenEffectSettings);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    #[fragment]
    pub fn fragment(_input: FragmentInput) -> Vec4f {
        let v = clamp(get!(SETTINGS).intensity / 10.0, 0.0, 1.0);
        vec4f(v, v, v, 1.0)
    }
}

/// Math helpers for EM interference.
#[wgsl]
pub mod em_interference_helpers {
    use wgsl_rs::std::*;

    pub fn rng2(seed: Vec2f, time: f32) -> f32 {
        let tick = floor(time / 0.07);
        fract(
            sin(dot(
                vec2f(tick, seed.x() + seed.y() * 23.0),
                vec2f(7.19, 13.37),
            )) * 43758.547,
        )
    }

    pub fn rng(seed: f32, time: f32) -> f32 {
        rng2(vec2f(seed, 1.0), time)
    }
}

#[wgsl]
pub mod em_interference {
    use super::em_interference_helpers::*;
    use super::fullscreen_settings::*;
    use wgsl_rs::std::*;

    texture!(group(0), binding(0), SCREEN_TEXTURE: Texture2D<f32>);
    sampler!(group(0), binding(1), TEXTURE_SAMPLER: Sampler);
    uniform!(group(0), binding(2), SETTINGS: FullscreenEffectSettings);
    uniform!(group(1), binding(0), GLOBALS: GlobalsUniform);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        let intensity = get!(SETTINGS).intensity;
        let t = get!(GLOBALS).time % 6000.0;

        let dims = texture_dimensions(SCREEN_TEXTURE);
        let resolution = vec2f(dims.x() as f32, dims.y() as f32);
        let uv = input.position.xy() / resolution;

        let block_s = floor(uv * vec2f(24.0, 9.0));
        let block_l = floor(uv * vec2f(8.0, 4.0));

        let r = rng2(uv, t);
        let noise = (vec3f(r, 1.0 - r, r / 2.0 + 0.5) - vec3f(2.0, 2.0, 2.0)) * 0.08;

        let line_noise = pow(rng2(block_s, t), 8.0) * pow(rng2(block_l, t), 3.0)
            - pow(rng(7.2341, t), 17.0) * 2.0;

        let col1 = texture_sample(SCREEN_TEXTURE, TEXTURE_SAMPLER, uv);
        let col2 = texture_sample(
            SCREEN_TEXTURE,
            TEXTURE_SAMPLER,
            uv + vec2f(line_noise * 0.05 * rng(5.0, t) * intensity, 0.0),
        );
        let col3 = texture_sample(
            SCREEN_TEXTURE,
            TEXTURE_SAMPLER,
            uv - vec2f(line_noise * 0.05 * rng(31.0, t) * intensity, 0.0),
        );

        vec4f(
            col1.x() + noise.x() * intensity,
            col2.y() + noise.y() * intensity,
            col3.z() + noise.z() * intensity,
            1.0,
        )
    }
}

#[wgsl]
pub mod em_interference_test {
    use super::fullscreen_settings::*;
    use wgsl_rs::std::*;

    uniform!(group(0), binding(0), SETTINGS: FullscreenEffectSettings);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    #[fragment]
    pub fn fragment(_input: FragmentInput) -> Vec4f {
        let v = clamp(get!(SETTINGS).intensity / 10.0, 0.0, 1.0);
        vec4f(v, v, v, 1.0)
    }
}

#[allow(clippy::approx_constant)]
#[wgsl]
pub mod oil_painting {
    use super::fullscreen_settings::*;
    use wgsl_rs::std::*;

    texture!(group(0), binding(0), SCREEN_TEXTURE: Texture2D<f32>);
    sampler!(group(0), binding(1), TEXTURE_SAMPLER: Sampler);
    uniform!(group(0), binding(2), SETTINGS: FullscreenEffectSettings);
    uniform!(group(1), binding(0), GLOBALS: GlobalsUniform);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    pub fn get_val(uv: Vec2f) -> f32 {
        let col = texture_sample_level(SCREEN_TEXTURE, TEXTURE_SAMPLER, uv, 0.0);
        length(vec3f(col.x(), col.y(), col.z()))
    }

    pub fn get_grad(uv: Vec2f, delta: f32) -> Vec2f {
        let dx = vec2f(delta, 0.0);
        let dy = vec2f(0.0, delta);
        vec2f(
            get_val(uv + dx) - get_val(uv - dx),
            get_val(uv + dy) - get_val(uv - dy),
        ) / delta
    }

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        let dims = texture_dimensions(SCREEN_TEXTURE);
        let res = vec2f(dims.x() as f32, dims.y() as f32);
        let uv = input.position.xy() / res;

        let grad = get_grad(uv, 3.0 / res.y());
        let n = normalize(vec3f(grad.x(), grad.y(), 30.0));

        let light = normalize(vec3f(-1.0, 1.0, 1.4));
        let diff = clamp(dot(n, light), 0.0, 1.0);

        let spec_raw = clamp(dot(reflect(light, n), vec3f(0.0, 0.0, -1.0)), 0.0, 1.0);
        let spec = pow(spec_raw, 12.0) * 0.15;

        let sh_raw = clamp(
            dot(
                reflect(light * vec3f(-1.0, -1.0, 1.0), n),
                vec3f(0.0, 0.0, -1.0),
            ),
            0.0,
            1.0,
        );
        let sh = pow(sh_raw, 4.0) * 0.1;

        let base = texture_sample(SCREEN_TEXTURE, TEXTURE_SAMPLER, uv);
        let highlight = vec4f(0.85, 1.0, 1.15, 1.0);
        let lit = base * mix(diff, 1.0, 0.9) + spec * highlight + sh * highlight;

        let vign_strength = 1.5;
        let scc = (input.position.xy() - 0.5 * res) / res.x();
        let mut vign = 1.1 - vign_strength * dot(scc, scc);
        vign *= 1.0 - 0.7 * vign_strength * exp(-sin(uv.x() * 3.1415927) * 40.0);
        vign *= 1.0 - 0.7 * vign_strength * exp(-sin(uv.y() * 3.1415927) * 20.0);

        vec4f(lit.x() * vign, lit.y() * vign, lit.z() * vign, 1.0)
    }
}

#[wgsl]
pub mod oil_painting_test {
    use super::fullscreen_settings::*;
    use wgsl_rs::std::*;

    uniform!(group(0), binding(0), SETTINGS: FullscreenEffectSettings);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    #[fragment]
    pub fn fragment(_input: FragmentInput) -> Vec4f {
        let v = clamp(get!(SETTINGS).intensity / 10.0, 0.0, 1.0);
        vec4f(v, v, v, 1.0)
    }
}

#[wgsl]
pub mod edge_cartoon_helpers {
    use wgsl_rs::std::*;

    pub fn rgb2lum(rgb: Vec3f) -> f32 {
        dot(rgb, vec3f(0.299, 0.587, 0.114))
    }

    pub fn cartoon_shading(color: Vec3f, intensity: f32) -> Vec3f {
        let lum = rgb2lum(color);
        let levels = mix(8.0, 3.0, intensity);
        let quantized_lum = floor(lum * levels) / levels;
        let scale = (quantized_lum + 0.1) / (lum + 0.1);
        color * scale
    }
}

#[wgsl]
pub mod edge_cartoon {
    use super::edge_cartoon_helpers::*;
    use super::fullscreen_settings::*;
    use wgsl_rs::std::*;

    texture!(group(0), binding(0), SCREEN_TEXTURE: Texture2D<f32>);
    sampler!(group(0), binding(1), TEXTURE_SAMPLER: Sampler);
    uniform!(group(0), binding(2), SETTINGS: FullscreenEffectSettings);
    uniform!(group(1), binding(0), GLOBALS: GlobalsUniform);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    pub fn calculate_sobel(uv: Vec2f, texel_size: Vec2f, edge_size: f32) -> Vec2f {
        let dirs: [Vec2f; 9] = [
            vec2f(-1.0, 1.0),
            vec2f(0.0, 1.0),
            vec2f(1.0, 1.0),
            vec2f(-1.0, 0.0),
            vec2f(0.0, 0.0),
            vec2f(1.0, 0.0),
            vec2f(-1.0, -1.0),
            vec2f(0.0, -1.0),
            vec2f(1.0, -1.0),
        ];
        let sobel_x: [f32; 9] = [-1.0, 0.0, 1.0, -2.0, 0.0, 2.0, -1.0, 0.0, 1.0];
        let sobel_y: [f32; 9] = [1.0, 2.0, 1.0, 0.0, 0.0, 0.0, -1.0, -2.0, -1.0];

        let center_lum = rgb2lum(texture_sample(SCREEN_TEXTURE, TEXTURE_SAMPLER, uv).xyz());
        let mut grad_x: f32 = 0.0;
        let mut grad_y: f32 = 0.0;

        let mut i: i32 = 0;
        while i < 9 {
            let s_uv = uv + dirs[i as usize] * edge_size * texel_size;
            let lum_diff =
                abs(center_lum
                    - rgb2lum(texture_sample(SCREEN_TEXTURE, TEXTURE_SAMPLER, s_uv).xyz()));
            grad_x += lum_diff * sobel_x[i as usize];
            grad_y += lum_diff * sobel_y[i as usize];
            i += 1;
        }
        vec2f(grad_x, grad_y)
    }

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        let intensity = get!(SETTINGS).intensity;
        let dims = texture_dimensions(SCREEN_TEXTURE);
        let resolution = vec2f(dims.x() as f32, dims.y() as f32);
        let texel_size = vec2f(1.0, 1.0) / resolution;
        let uv = input.position.xy() / resolution;

        let original = texture_sample(SCREEN_TEXTURE, TEXTURE_SAMPLER, uv).xyz();

        let line_size = mix(0.5, 1.2, intensity);
        let sobel = calculate_sobel(uv, texel_size, line_size);
        let edge_threshold: f32 = 0.01;
        let edge_factor = 1.0 - step(edge_threshold, length(sobel));

        let cartoon = cartoon_shading(original, intensity);
        let effect = mix(
            vec3f(0.0, 0.0, 0.0),
            cartoon,
            vec3f(edge_factor, edge_factor, edge_factor),
        );
        let final_color = mix(original, effect, vec3f(intensity, intensity, intensity));
        vec4f(final_color.x(), final_color.y(), final_color.z(), 1.0)
    }
}

#[wgsl]
pub mod edge_cartoon_test {
    use super::fullscreen_settings::*;
    use wgsl_rs::std::*;

    uniform!(group(0), binding(0), SETTINGS: FullscreenEffectSettings);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    #[fragment]
    pub fn fragment(_input: FragmentInput) -> Vec4f {
        let v = clamp(get!(SETTINGS).intensity / 10.0, 0.0, 1.0);
        vec4f(v, v, v, 1.0)
    }
}

#[wgsl]
pub mod cartoon_filter_helpers {
    use wgsl_rs::std::*;

    pub fn rgb2hsv(c: Vec3f) -> Vec3f {
        let k = vec4f(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
        let s_zy = step(c.z(), c.y());
        let p = mix(
            vec4f(c.z(), c.y(), k.w(), k.z()),
            vec4f(c.y(), c.z(), k.x(), k.y()),
            vec4f(s_zy, s_zy, s_zy, s_zy),
        );
        let s_px = step(p.x(), c.x());
        let q = mix(
            vec4f(p.x(), p.y(), p.w(), c.x()),
            vec4f(c.x(), p.y(), p.z(), p.x()),
            vec4f(s_px, s_px, s_px, s_px),
        );
        let d = q.x() - min(q.w(), q.y());
        let e: f32 = 1.0e-10;
        vec3f(
            abs(q.z() + (q.w() - q.y()) / (6.0 * d + e)),
            d / (q.x() + e),
            q.x(),
        )
    }

    pub fn hsv2rgb(c: Vec3f) -> Vec3f {
        let k = vec4f(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
        let p = abs(fract(c.xxx() + k.xyz()) * 6.0 - k.www());
        let t = mix(
            k.xxx(),
            clamp(p - k.xxx(), vec3f(0.0, 0.0, 0.0), vec3f(1.0, 1.0, 1.0)),
            vec3f(c.y(), c.y(), c.y()),
        );
        t * c.z()
    }

    pub fn dither_noise(coord: Vec2f) -> f32 {
        fract(sin(dot(coord, vec2f(12.9898, 78.233))) * 43758.547)
    }

    pub fn quantize_rgb(rgb: Vec3f, res: f32, coord: Vec2f) -> Vec3f {
        let c1 = floor(rgb * res) / res;
        let c2 = (floor(rgb * res) + vec3f(1.0, 1.0, 1.0)) / res;
        let dith = dither_noise(coord);
        let a = step(dith, distance(c1, rgb)); // scalar: 0.0 or 1.0
        mix(c1, c2, vec3f(a, a, a))
    }
}

#[wgsl]
pub mod cartoon_filter {
    use super::cartoon_filter_helpers::*;
    use super::fullscreen_settings::*;
    use wgsl_rs::std::*;

    texture!(group(0), binding(0), SCREEN_TEXTURE: Texture2D<f32>);
    sampler!(group(0), binding(1), TEXTURE_SAMPLER: Sampler);
    uniform!(group(0), binding(2), SETTINGS: FullscreenEffectSettings);
    uniform!(group(1), binding(0), GLOBALS: GlobalsUniform);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    pub fn sample_lum(uv: Vec2f, offset: Vec2f) -> f32 {
        rgb2hsv(texture_sample(SCREEN_TEXTURE, TEXTURE_SAMPLER, uv + offset).xyz()).z()
    }

    pub fn gradient_magnitude(uv: Vec2f, texel_size: Vec2f) -> f32 {
        let edge_thickness: f32 = 1.5;
        let ts = texel_size * edge_thickness;
        let tl = sample_lum(uv, vec2f(-ts.x(), ts.y()));
        let tc = sample_lum(uv, vec2f(0.0, ts.y()));
        let tr = sample_lum(uv, vec2f(ts.x(), ts.y()));
        let ml = sample_lum(uv, vec2f(-ts.x(), 0.0));
        let mr = sample_lum(uv, vec2f(ts.x(), 0.0));
        let bl = sample_lum(uv, vec2f(-ts.x(), -ts.y()));
        let bc = sample_lum(uv, vec2f(0.0, -ts.y()));
        let br = sample_lum(uv, vec2f(ts.x(), -ts.y()));
        let gx = -tl - 2.0 * ml - bl + tr + 2.0 * mr + br;
        let gy = -tl - 2.0 * tc - tr + bl + 2.0 * bc + br;
        sqrt(gx * gx + gy * gy)
    }

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        let color_levels: f32 = 4.0;
        let edge_threshold: f32 = 0.1;
        let dither_scale: f32 = 2.0;

        let intensity = get!(SETTINGS).intensity;
        let dims = texture_dimensions(SCREEN_TEXTURE);
        let resolution = vec2f(dims.x() as f32, dims.y() as f32);
        let texel_size = vec2f(1.0, 1.0) / resolution;
        let uv = input.position.xy() / resolution;

        let original = texture_sample(SCREEN_TEXTURE, TEXTURE_SAMPLER, uv).xyz();
        let levels = mix(16.0, color_levels, intensity); // scalar mix - ok
        let quantized = quantize_rgb(original, levels, uv * resolution / dither_scale);
        let edge = gradient_magnitude(uv, texel_size);
        let edge_str = smoothstep(edge_threshold * 0.5, edge_threshold * 2.0, edge);
        let cartoon = mix(
            quantized,
            vec3f(0.0, 0.0, 0.0),
            vec3f(edge_str, edge_str, edge_str),
        );

        let mut hsv = rgb2hsv(cartoon);
        let sat_boosted = min(hsv.y() * 1.3, 1.0);
        hsv = vec3f(hsv.x(), mix(hsv.y(), sat_boosted, intensity), hsv.z());
        let boosted = hsv2rgb(hsv);

        let final_color = mix(original, boosted, vec3f(intensity, intensity, intensity));
        vec4f(final_color.x(), final_color.y(), final_color.z(), 1.0)
    }
}

#[wgsl]
pub mod cartoon_filter_test {
    use super::fullscreen_settings::*;
    use wgsl_rs::std::*;

    uniform!(group(0), binding(0), SETTINGS: FullscreenEffectSettings);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    #[fragment]
    pub fn fragment(_input: FragmentInput) -> Vec4f {
        let v = clamp(get!(SETTINGS).intensity / 10.0, 0.0, 1.0);
        vec4f(v, v, v, 1.0)
    }
}
