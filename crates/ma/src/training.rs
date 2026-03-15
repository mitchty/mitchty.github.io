use crate::{
    data::{DataItem, MnistBatch, MnistBatcher, NormStats},
    model::{Model, ModelConfig},
};
use burn::{
    data::{dataloader::DataLoaderBuilder, dataset::Dataset},
    nn::loss::CrossEntropyLossConfig,
    optim::AdamConfig,
    prelude::*,
    record::CompactRecorder,
    tensor::backend::AutodiffBackend,
    train::{
        ClassificationOutput, InferenceStep, Learner, SupervisedTraining, TrainOutput, TrainStep,
        metric::{AccuracyMetric, LossMetric},
    },
};

impl<B: Backend> Model<B> {
    pub fn forward_classification(
        &self,
        images: Tensor<B, 3>,
        targets: Tensor<B, 1, Int>,
    ) -> ClassificationOutput<B> {
        let output = self.forward(images);
        let loss = CrossEntropyLossConfig::new()
            .init(&output.device())
            .forward(output.clone(), targets.clone());

        ClassificationOutput::new(loss, output, targets)
    }
}

impl<B: AutodiffBackend> TrainStep for Model<B> {
    type Input = MnistBatch<B>;
    type Output = ClassificationOutput<B>;

    fn step(&self, batch: MnistBatch<B>) -> TrainOutput<ClassificationOutput<B>> {
        let item = self.forward_classification(batch.images, batch.targets);

        TrainOutput::new(self, item.loss.backward(), item)
    }
}

impl<B: Backend> InferenceStep for Model<B> {
    type Input = MnistBatch<B>;
    type Output = ClassificationOutput<B>;

    fn step(&self, batch: MnistBatch<B>) -> ClassificationOutput<B> {
        self.forward_classification(batch.images, batch.targets)
    }
}

#[derive(Config, Debug)]
pub struct TrainingConfig {
    pub model: ModelConfig,
    pub optimizer: AdamConfig,
    #[config(default = 30)]
    pub num_epochs: usize,
    #[config(default = 64)]
    pub batch_size: usize,
    #[config(default = 4)]
    pub num_workers: usize,
    #[config(default = 42)]
    pub seed: u64,
    #[config(default = 1.0e-4)]
    pub learning_rate: f64,
    /// Per-channel pixel mean used for normalisation: (pixel/255 - mean) / std.
    /// Saved to config.json and must match the inference engine's stats exactly.
    /// Default: K49 dataset mean (0.1793).
    #[config(default = 0.1793)]
    pub norm_mean: f64,
    /// Per-channel pixel std used for normalisation.
    /// Default: K49 dataset std (0.3416).
    #[config(default = 0.3416)]
    pub norm_std: f64,
    /// Class index -> Unicode character mapping, in label-index order.
    ///
    /// Populated at training time from a `classmap.json` written by
    /// `ma convert`.  Saved to `config.json` so the inference engine can
    /// display the correct character for each predicted class without any
    /// separate lookup file.  Empty for models trained without a classmap
    /// (e.g. built-in MNIST fallback).
    #[config(default = "Vec::new()")]
    pub class_map: Vec<String>,
}

fn create_artifact_dir(artifact_dir: &str) {
    // Remove existing artifacts before to get an accurate learner summary
    std::fs::remove_dir_all(artifact_dir).ok();
    std::fs::create_dir_all(artifact_dir).ok();
}

pub fn train<B, D>(
    artifact_dir: &str,
    config: TrainingConfig,
    device: B::Device,
    train_dataset: D,
    test_dataset: D,
) where
    B: AutodiffBackend,
    D: Dataset<DataItem> + 'static,
{
    create_artifact_dir(artifact_dir);
    config
        .save(format!("{artifact_dir}/config.json"))
        .expect("Config should be saved successfully");

    B::seed(&device, config.seed);

    let norm = NormStats {
        mean: config.norm_mean as f32,
        std: config.norm_std as f32,
    };

    // Training batcher has augmentation on; validation batcher does not so
    // that validation metrics are computed on clean, un-transformed images.
    let batcher_train = MnistBatcher::new(norm).with_augment(true);
    let batcher_valid = MnistBatcher::new(norm);

    let dataloader_train = DataLoaderBuilder::new(batcher_train)
        .batch_size(config.batch_size)
        .shuffle(config.seed)
        .num_workers(config.num_workers)
        .build(train_dataset);

    let dataloader_test = DataLoaderBuilder::new(batcher_valid)
        .batch_size(config.batch_size)
        .shuffle(config.seed)
        .num_workers(config.num_workers)
        .build(test_dataset);

    let training = SupervisedTraining::new(artifact_dir, dataloader_train, dataloader_test)
        .metrics((AccuracyMetric::new(), LossMetric::new()))
        .with_file_checkpointer(CompactRecorder::new())
        .num_epochs(config.num_epochs)
        .summary();

    let model = config.model.init::<B>(&device);
    let result = training.launch(Learner::new(
        model,
        config.optimizer.init(),
        config.learning_rate,
    ));

    result
        .model
        .save_file(format!("{artifact_dir}/model"), &CompactRecorder::new())
        .expect("Trained model should be saved successfully");
}
