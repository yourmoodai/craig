"""Export local BERT weights; synthetic tracing only, no runtime Python dependency."""
import argparse
import hashlib
import json
import types
from pathlib import Path

import coremltools as ct
import numpy as np
import torch
import transformers
from transformers import BertForTokenClassification


def digest(path):
    return hashlib.file_digest(path.open("rb"), "sha256").hexdigest()


class Classifier(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, features):
        return self.model(input_ids=features[0], attention_mask=features[1],
                          token_type_ids=features[2]).logits


def finite_attention_mask(self, attention_mask, input_shape, device=None, dtype=None):
    # Keep masked logits finite through FP16 conversion; exp(-10000) is zero in both precisions.
    return (1.0 - attention_mask[:, None, None, :].to(dtype=self.dtype)) * -10000.0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", type=Path, default=Path(".cache/models/mudes-en-base"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--precision", choices=["float32", "float16"], default="float32")
    parser.add_argument("--fixed-batch", type=int, choices=[1, 2, 4, 8])
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output_exists")
    model = BertForTokenClassification.from_pretrained(
        args.model, local_files_only=True, attn_implementation="eager").eval()
    model.bert.get_extended_attention_mask = types.MethodType(finite_attention_mask, model.bert)
    head = args.model / "head.safetensors"
    if head.exists() != (args.model / "manifest.json").exists():
        parser.error("incomplete_candidate")
    if head.exists():
        manifest = json.loads((args.model / "manifest.json").read_text())
        if manifest["head_sha256"] != digest(head):
            parser.error("head_digest_mismatch")
        from safetensors.torch import load_file
        state = load_file(head)
        model.classifier.load_state_dict({k.removeprefix("classifier."): v for k, v in state.items()})
    sample = torch.zeros((3, args.fixed_batch or 1, 512), dtype=torch.int32)
    sample[1] = 1
    with torch.no_grad():
        traced = torch.jit.trace(Classifier(model).eval(), sample)
    shapes = [(3, args.fixed_batch, 512)] if args.fixed_batch else [(3, b, w) for b in [1, 2, 4, 8] for w in [16, 32, 64, 128, 256, 512]]
    shape = shapes[0] if args.fixed_batch else ct.EnumeratedShapes(shapes, default=(3, 1, 512))
    converted = ct.convert(
        traced, convert_to="mlprogram", minimum_deployment_target=ct.target.macOS15,
        inputs=[ct.TensorType(name="features", dtype=np.int32,
                             shape=shape)],
        outputs=[ct.TensorType(name="logits", dtype=np.float32)],
        compute_precision=ct.precision.FLOAT16 if args.precision == "float16" else ct.precision.FLOAT32,
        skip_model_load=True,
    )
    args.output.mkdir(parents=True)
    package = args.output / "classifier.mlpackage"
    converted.save(str(package))
    sources = {p.name: digest(p) for p in args.model.iterdir()
               if p.name in ["pytorch_model.bin", "config.json", "vocab.txt", "head.safetensors", "manifest.json"]}
    files = {str(p.relative_to(args.output)): digest(p) for p in sorted(package.rglob("*")) if p.is_file()}
    manifest = {"version": 1, "precision": args.precision, "sources": sources, "files": files,
                "exporter_sha256": digest(Path(__file__)),
                "coremltools": ct.__version__, "torch": torch.__version__, "numpy": np.__version__, "transformers": transformers.__version__, "shapes": shapes}
    (args.output / "export.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print("local core ml export written")


if __name__ == "__main__":
    main()
