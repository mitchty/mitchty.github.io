#![allow(dead_code)]

use crate::ai::reparameterize;
use burn::{
    config::Config,
    module::Module,
    nn::{Linear, LinearConfig, Relu},
    tensor::{Tensor, activation, backend::Backend},
};

/// Configuration for the Dense Variational Autoencoder.
#[derive(Config, Debug)]
pub struct DenseVaeConfig {
    /// The size of the input feature vector
    #[config(default = 784)]
    pub input_dim: usize,
    /// A list defining the topology of the hidden layers in the Encoder.
    #[config(default = "vec![512, 256, 64]")]
    pub hidden_dims: Vec<usize>,
    /// The dimension of the latent space $z$.
    #[config(default = 20)]
    pub latent_dim: usize,
    /// Learning rate for the optimizer.
    #[config(default = 1e-3)]
    pub learning_rate: f64,
    /// Total number of training epochs.
    #[config(default = 25)]
    pub num_epochs: usize,
    /// Mini-batch size for training and inference.
    #[config(default = 128)]
    pub batch_size: usize,
}

/// The Probabilistic Encoder $q_\phi(z|x)$.
#[derive(Module, Debug)]
pub struct DenseEncoder<B: Backend> {
    /// Dynamic stack of fully connected hidden layers.
    layers: Vec<Linear<B>>,
    /// Linear head to predict the mean ($\mu$) of the latent distribution.
    fc_mu: Linear<B>,
    /// Linear head to predict the log-variance ($\log \sigma^2$) of the latent distribution.
    fc_logvar: Linear<B>,
    /// Activation function applied after every hidden layer.
    activation: Relu,
}

impl<B: Backend> DenseEncoder<B> {
    pub fn new(config: &DenseVaeConfig, device: &B::Device) -> Self {
        let mut layers = Vec::new();
        let mut current_dim = config.input_dim;

        for &dim in &config.hidden_dims {
            layers.push(LinearConfig::new(current_dim, dim).init(device));
            current_dim = dim;
        }

        let fc_mu = LinearConfig::new(current_dim, config.latent_dim).init(device);
        let fc_logvar = LinearConfig::new(current_dim, config.latent_dim).init(device);

        Self {
            layers,
            fc_mu,
            fc_logvar,
            activation: Relu::new(),
        }
    }

    pub fn forward(&self, x: Tensor<B, 2>) -> (Tensor<B, 2>, Tensor<B, 2>) {
        let mut x = x;

        for layer in &self.layers {
            x = layer.forward(x);
            x = self.activation.forward(x);
        }

        let mu = self.fc_mu.forward(x.clone());
        let logvar = self.fc_logvar.forward(x);

        (mu, logvar)
    }
}

/// The Probabilistic Decoder $p_\theta(x|z)$.
#[derive(Module, Debug)]
pub struct DenseDecoder<B: Backend> {
    /// Dynamic stack of fully connected hidden layers in reversed topology.
    layers: Vec<Linear<B>>,
    /// Final projection layer to the original input dimension.
    output_layer: Linear<B>,
    /// Activation function applied after hidden layers.
    activation: Relu,
}

impl<B: Backend> DenseDecoder<B> {
    /// Iterates through `hidden_dims` in reverse to create a mirror image of the encoder.
    pub fn new(config: &DenseVaeConfig, device: &B::Device) -> Self {
        let mut layers = Vec::new();
        let mut current_dim = config.latent_dim;

        // NOte iterate in reverse to the Encoder
        for &dim in config.hidden_dims.iter().rev() {
            layers.push(LinearConfig::new(current_dim, dim).init(device));
            current_dim = dim;
        }

        let output_layer = LinearConfig::new(current_dim, config.input_dim).init(device);

        Self {
            layers,
            output_layer,
            activation: Relu::new(),
        }
    }

    /// Performs the forward pass of the decoder.
    pub fn forward(&self, z: Tensor<B, 2>) -> Tensor<B, 2> {
        let mut x = z;

        for layer in &self.layers {
            x = layer.forward(x);
            x = self.activation.forward(x);
        }

        let x = self.output_layer.forward(x);
        activation::sigmoid(x)
    }
}

/// The complete Variational Autoencoder module.
#[derive(Module, Debug)]
pub struct DenseVAE<B: Backend> {
    pub encoder: DenseEncoder<B>,
    pub decoder: DenseDecoder<B>,
}

impl<B: Backend> DenseVAE<B> {
    pub fn new(config: &DenseVaeConfig, device: &B::Device) -> Self {
        Self {
            encoder: DenseEncoder::new(config, device),
            decoder: DenseDecoder::new(config, device),
        }
    }

    /// The full forward pass of the VAE.
    ///
    /// # Steps
    /// 1. **Encode**: Map input `x` to `mu` and `logvar`.
    /// 2. **Reparameterize**: Sample `z = mu + sigma * epsilon`.
    /// 3. **Decode**: Reconstruct `recon_x` from `z`.
    ///
    /// # Arguments
    /// * `x` j= Input tensor. Shape: `(Batch, Input_Dim)`.
    ///
    /// # Returns
    /// A tuple containing:
    /// 1. `recon_x`: Reconstructed input. Shape: `(Batch, Input_Dim)`.
    /// 2. `mu`: Latent mean. Shape: `(Batch, Latent_Dim)`.
    /// 3. `logvar`: Latent log-variance. Shape: `(Batch, Latent_Dim)`.
    ///
    /// These three tensors are required to compute the VAE Loss, ELBO.
    pub fn forward(&self, x: Tensor<B, 2>) -> (Tensor<B, 2>, Tensor<B, 2>, Tensor<B, 2>) {
        let (mu, logvar) = self.encoder.forward(x);
        let z = reparameterize(mu.clone(), logvar.clone());
        let recon_x = self.decoder.forward(z);
        (recon_x, mu, logvar)
    }
}
