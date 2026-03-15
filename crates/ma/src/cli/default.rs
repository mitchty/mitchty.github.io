use burn::backend::Wgpu;

use crate::model::ModelConfig;

pub fn run() {
    crate::cli::init_tracing();
    type MyBackend = Wgpu<f32, i32>;

    let device = Default::default();
    let model = ModelConfig::new(10, 512).init::<MyBackend>(&device);

    tracing::info!(model = ?model, "model architecture");
}
