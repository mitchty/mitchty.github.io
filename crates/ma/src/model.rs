use burn::{
    nn::{
        BatchNorm, BatchNormConfig, Dropout, DropoutConfig, Linear, LinearConfig, Relu,
        conv::{Conv2d, Conv2dConfig},
        pool::{AdaptiveAvgPool2d, AdaptiveAvgPool2dConfig},
    },
    prelude::*,
};

#[derive(Module, Debug)]
pub struct Model<B: Backend> {
    conv1: Conv2d<B>,
    bn1: BatchNorm<B>,
    conv2: Conv2d<B>,
    bn2: BatchNorm<B>,
    conv3: Conv2d<B>,
    bn3: BatchNorm<B>,
    pool: AdaptiveAvgPool2d,
    dropout: Dropout,
    linear1: Linear<B>,
    linear2: Linear<B>,
    activation: Relu,
}

#[derive(Config, Debug)]
pub struct ModelConfig {
    num_classes: usize,
    hidden_size: usize,
    #[config(default = "0.5")]
    dropout: f64,
    /// Output channels after conv1. conv2 doubles this, conv3 doubles again.
    /// Default 32 gives 32->64->128 to help keep gpu compute units busier.
    #[config(default = "32")]
    pub conv_channels: usize,
}

impl ModelConfig {
    /// Returns the initialized model.
    ///
    /// Architecture:
    ///   conv1 (1 -> C, 3x3, pad=1) -> BN -> ReLU
    ///   conv2 (C -> 2C, 3x3, pad=1) -> BN -> ReLU
    ///   conv3 (2C -> 4C, 3x3, pad=1) -> BN -> ReLU
    ///   AdaptiveAvgPool -> [B, 4C, 4, 4]
    ///   Linear(4C*4*4 -> hidden_size) -> Dropout -> ReLU
    ///   Linear(hidden_size -> num_classes)
    ///
    /// With C=32 (default): 32->64->128, pool->\[B,128,4,4\]=2048 -> hidden -> classes.
    /// With C=64:           64->128->256, pool->\[B,256,4,4\]=4096 -> hidden -> classes.
    pub fn init<B: Backend>(&self, device: &B::Device) -> Model<B> {
        let c = self.conv_channels;
        Model {
            conv1: Conv2dConfig::new([1, c], [3, 3])
                .with_padding(burn::nn::PaddingConfig2d::Same)
                .init(device),
            bn1: BatchNormConfig::new(c).init(device),
            conv2: Conv2dConfig::new([c, c * 2], [3, 3])
                .with_padding(burn::nn::PaddingConfig2d::Same)
                .init(device),
            bn2: BatchNormConfig::new(c * 2).init(device),
            conv3: Conv2dConfig::new([c * 2, c * 4], [3, 3])
                .with_padding(burn::nn::PaddingConfig2d::Same)
                .init(device),
            bn3: BatchNormConfig::new(c * 4).init(device),
            pool: AdaptiveAvgPool2dConfig::new([4, 4]).init(),
            activation: Relu::new(),
            linear1: LinearConfig::new(c * 4 * 4 * 4, self.hidden_size).init(device),
            linear2: LinearConfig::new(self.hidden_size, self.num_classes).init(device),
            dropout: DropoutConfig::new(self.dropout).init(),
        }
    }
}

impl<B: Backend> Model<B> {
    /// # Possible Shapes
    ///   - Images [batch_size, height, width]
    ///   - Output [batch_size, num_classes]
    pub fn forward(&self, images: Tensor<B, 3>) -> Tensor<B, 2> {
        let [batch_size, height, width] = images.dims();

        // Add channel dim: [B, 1, H, W]
        let x = images.reshape([batch_size, 1, height, width]);

        // Block 1
        let x = self.conv1.forward(x);
        let x = self.bn1.forward(x);
        let x = self.activation.forward(x);

        // Block 2
        let x = self.conv2.forward(x);
        let x = self.bn2.forward(x);
        let x = self.activation.forward(x);
        let x = self.dropout.forward(x);

        // Block 3
        let x = self.conv3.forward(x);
        let x = self.bn3.forward(x);
        let x = self.activation.forward(x);
        let x = self.dropout.forward(x);

        // Pool + flatten: [B, 4C, 4, 4] -> [B, 4C*16]
        let x = self.pool.forward(x);
        let features = x.dims()[1] * x.dims()[2] * x.dims()[3];
        let x = x.reshape([batch_size, features]);

        // Classifier head
        let x = self.linear1.forward(x);
        let x = self.dropout.forward(x);
        let x = self.activation.forward(x);

        self.linear2.forward(x)
    }
}
