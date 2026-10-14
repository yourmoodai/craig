use candle_core::{DType, Module, Result, Tensor};
use candle_nn::{Embedding, LayerNorm, VarBuilder, embedding, layer_norm};
use candle_transformers::models::bert::{BertEncoder, Config};

/// Inference embeddings and a finite padding mask; upstream owns encoder layers.
pub struct Bert {
    words: Embedding,
    positions: Embedding,
    types: Embedding,
    norm: LayerNorm,
    encoder: BertEncoder,
}
impl Bert {
    pub fn load(weights: VarBuilder<'_>, config: &Config) -> Result<Self> {
        let e = weights.pp("embeddings");
        Ok(Self {
            words: embedding(
                config.vocab_size,
                config.hidden_size,
                e.pp("word_embeddings"),
            )?,
            positions: embedding(
                config.max_position_embeddings,
                config.hidden_size,
                e.pp("position_embeddings"),
            )?,
            types: embedding(
                config.type_vocab_size,
                config.hidden_size,
                e.pp("token_type_embeddings"),
            )?,
            norm: layer_norm(config.hidden_size, config.layer_norm_eps, e.pp("LayerNorm"))?,
            encoder: BertEncoder::load(weights.pp("encoder"), config)?,
        })
    }
    pub fn forward(&self, ids: &Tensor, types: &Tensor, mask: &Tensor) -> Result<Tensor> {
        let (_, width) = ids.dims2()?;
        let positions = Tensor::arange(0u32, width as u32, ids.device())?;
        let hidden = (self.words.forward(ids)? + self.types.forward(types)?)?
            .broadcast_add(&self.positions.forward(&positions)?)?;
        let hidden = self.norm.forward(&hidden)?;
        self.encoder
            .forward(&hidden, &attention_mask(mask, hidden.dtype())?)
    }
}
fn attention_mask(mask: &Tensor, dtype: DType) -> Result<Tensor> {
    // f32::MIN becomes -infinity in FP16, yielding NaN for visible tokens (0 * -inf).
    let minimum = if dtype == DType::F16 {
        -65504.0f32
    } else {
        f32::MIN
    };
    let mask = mask.unsqueeze(1)?.unsqueeze(1)?.to_dtype(dtype)?;
    (mask.ones_like()? - &mask)?
        .broadcast_mul(&Tensor::new(minimum, mask.device())?.to_dtype(dtype)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn half_mask_has_finite_visible_and_padding_values() {
        let mask = Tensor::new(&[[1u32, 0]], &candle_core::Device::Cpu).unwrap();
        let values = attention_mask(&mask, DType::F16)
            .unwrap()
            .flatten_all()
            .unwrap()
            .to_dtype(DType::F32)
            .unwrap()
            .to_vec1::<f32>()
            .unwrap();
        assert_eq!(values, [0.0, -65504.0]);
    }
}
