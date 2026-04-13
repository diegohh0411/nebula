import sys
import json
import torch
from transformers import AutoModel, AutoProcessor
from PIL import Image

CHECKPOINT = "google/siglip2-so400m-patch16-naflex"


def main():
    # Signal that model loading has started
    print(json.dumps({"status": "loading", "action": "ready"}), flush=True)

    model = AutoModel.from_pretrained(CHECKPOINT).eval()
    processor = AutoProcessor.from_pretrained(CHECKPOINT)
    device = "cuda" if torch.cuda.is_available() else "cpu"
    model = model.to(device)

    # Signal that model is ready
    print(json.dumps({"status": "ok", "action": "ready"}), flush=True)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        request = None
        try:
            request = json.loads(line)
            action = request.get("action")

            if action == "embed_image":
                image = Image.open(request["image_path"]).convert("RGB")
                inputs = processor(images=[image], return_tensors="pt").to(device)
                with torch.no_grad():
                    features = model.get_image_features(**inputs)
                features = features / features.norm(p=2, dim=-1, keepdim=True)
                embedding = features[0].cpu().tolist()
                print(json.dumps({
                    "status": "ok",
                    "action": "embed_image",
                    "image_path": request["image_path"],
                    "embedding": embedding,
                }), flush=True)

            elif action == "embed_text":
                text = f"This is a photo of {request['text'].lower()}."
                inputs = processor(
                    text=[text],
                    padding="max_length",
                    truncation=True,
                    max_length=64,
                    return_tensors="pt",
                ).to(device)
                with torch.no_grad():
                    features = model.get_text_features(**inputs)
                features = features / features.norm(p=2, dim=-1, keepdim=True)
                embedding = features[0].cpu().tolist()
                print(json.dumps({
                    "status": "ok",
                    "action": "embed_text",
                    "text": request["text"],
                    "embedding": embedding,
                }), flush=True)

            elif action == "health_check":
                print(json.dumps({
                    "status": "ok",
                    "action": "health_check",
                    "model": CHECKPOINT,
                }), flush=True)

            elif action == "shutdown":
                break

        except Exception as e:
            print(json.dumps({
                "status": "error",
                "action": request.get("action", "unknown") if request else "unknown",
                "message": str(e),
            }), flush=True)


if __name__ == "__main__":
    main()
