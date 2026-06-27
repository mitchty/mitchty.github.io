//! Canary fill shader of solid black everywhere. Just a dum fragment shader.
//! Used by the headless test harness to help validate if we get a clear color
//! when we don't expect to.
use wgsl_rs::wgsl;

#[wgsl]
pub mod canary_fill {
    use wgsl_rs::std::*;

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
    }

    #[fragment]
    pub fn fragment(_input: FragmentInput) -> Vec4f {
        vec4f(0.0, 0.0, 0.0, 1.0)
    }
}
