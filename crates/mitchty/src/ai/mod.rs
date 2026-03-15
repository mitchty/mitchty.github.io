use burn::{prelude::*, tensor::Distribution};

pub mod infer;
pub mod model;

#[allow(unused_imports)]
pub use infer::InferenceEngine;

// Re-export the reparameterization trick for use by model sub-modules.
/// Implements the Reparameterization Trick.
///
/// Allows backpropagation to flow through the stochastic sampling
/// node in the network. Instead of sampling `z ~ N(μ, σ²)` directly
/// (which is non-differentiable), we sample noise `ε ~ N(0, 1)` and compute:
///
/// z = μ + σ · ε
///
/// # Arguments
/// * `mu`     - The mean vector (μ).
/// * `logvar` - The log-variance vector (log(σ²)).
///
/// # Returns
/// A sampled latent vector `z` compatible with backpropagation.
#[allow(dead_code)]
pub fn reparameterize<B: Backend, const D: usize>(
    mu: Tensor<B, D>,
    logvar: Tensor<B, D>,
) -> Tensor<B, D> {
    let std = logvar.mul_scalar(0.5).exp();
    let eps = Tensor::random_like(&std, Distribution::Normal(0.0, 1.0));
    mu + eps * std
}
