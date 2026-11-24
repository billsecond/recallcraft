# The speech model

`whisper-base.en-q8_0.gguf` is what writes down what's said on calls: what you say into the
microphone and what the others say through the computer. RecallCraft runs it on this computer, in
a program of its own at below-normal priority (`recallcraft --hear <model>`); nothing is sent
anywhere, and it's never downloaded.

| | |
|---|---|
| Model | Whisper base, English only (74 million weights), from *Robust Speech Recognition via Large-Scale Weak Supervision* (Alec Radford, Jong Wook Kim, Tao Xu, Greg Brockman, Christine McLeavey, Ilya Sutskever; OpenAI, 2022) |
| Weights | `openai/whisper-base.en` as OpenAI publishes it on Hugging Face (https://huggingface.co/openai/whisper-base.en) |
| Licence | MIT (https://github.com/openai/whisper; the notice is `LICENSE-WHISPER.txt`, beside the model) |
| This file | 83.8 MB, SHA-256 `08c6a80bcb3b3ff8cae0b82413ae14d85cd4761c34093fe881f71b687bb7bd6e`, the same file ScreenGrabCraft installs |

Every matrix of weights is kept in 8 bits in blocks of 32 (`Q8_0`); the rest stays in 32-bit
floats, and the vocabulary is in the same file. The same Hugging Face files make the same model
file, byte for byte:

```sh
cargo run --release -p recallcraft-speech --example convert_model -- \
    <folder with model.safetensors, config.json, tokenizer.json> assets/models/whisper-base.en-q8_0.gguf
```

## How fast

A 54-second call with three voices (made-up sentences read by Windows' text-to-speech voices)
was written down in 4 seconds on two threads, every word right, each line given to the right
person (`cargo run --release -p recallcraft-engine --example simulate`).

## Another model

A `whisper….gguf` file put in the `models` folder in the settings folder
(`%APPDATA%\RecallCraft\models`) is used instead (the first by name). Another of OpenAI's
English-only models (`tiny.en`, `small.en`, `medium.en`) can be made with `convert_model`:
`small.en` hears better and needs about three times the processor.
