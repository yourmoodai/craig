"""Generate synthetic reference probabilities; never consumes user prompts."""
import json
import os
from pathlib import Path
import torch
from transformers import BertForTokenClassification, BertTokenizerFast

root = Path(__file__).resolve().parents[1]
model_dir = Path(os.environ.get("MIDDLE_FINGER_MODEL_DIR", root / ".cache/models/mudes-en-base"))
model = BertForTokenClassification.from_pretrained(model_dir, local_files_only=True).eval()
tokenizer = BertTokenizerFast(vocab_file=str(model_dir / "vocab.txt"), do_lower_case=False)
texts = ["You idiot, fix the test.", "Your previous answer was wrong.", "🙂 You stupid idiot, do not delete the test."]
results = []
for text in texts:
    encoded = tokenizer(text, return_offsets_mapping=True, return_tensors="pt")
    offsets = encoded.pop("offset_mapping")[0].tolist()
    with torch.no_grad():
        scores = model(**encoded).logits.softmax(-1)[0, :, 1].tolist()
    spans = [{"start":len(text[:a].encode()), "end":len(text[:b].encode()), "score":score} for (a,b),score in zip(offsets,scores) if a < b]
    results.append({"text":text,"spans":spans})
(root / ".cache/reference.json").write_text(json.dumps(results))
print("synthetic reference probabilities written")
