# The speech model

`whisper-base.en-q8_0.gguf` is what writes down what's said on calls: what you say into the
microphone and what the others say through the computer. RecallCraft runs it on this computer, in
a program of its own at below-normal priority (`recallcraft --hear <model>`); nothing is sent
anywhere. It's installed with RecallCraft; the larger Small and Medium are downloaded when
chosen (below).

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

## Larger models: Small and Medium

Settings › Calls › Speech model offers two larger English-only models, downloaded when chosen
from the repository's `speech-models` release (a pre-release, so updates never offer it) into
`models` in the data folder, and checked against the SHA-256 in `crates/desktop/src/models.rs`.
Until one is there, Base hears; the hearer takes it up between calls. Choosing another deletes
it.

| | Weights | File | Size | SHA-256 |
|---|---|---|---|---|
| Small | `openai/whisper-small.en` (244 million) | `whisper-small.en-q8_0.gguf` | 268.6 MB | `8fc18e0736d8b486dfe65a6f780cb75a8ca2684ab782e1d381c73160662ff326` |
| Medium | `openai/whisper-medium.en` (769 million) | `whisper-medium.en-q8_0.gguf` | 830.4 MB | `eccef047c9a63db6e6c38ae511c0771df8563d41c294ca762755a6b156d7aea5` |

Both are made by `convert_model` from the Hugging Face files, as Base is. The same made-up call,
on a 24-thread desktop processor: Base on two threads heard 56 seconds in 5.8 s; Small on three,
11.9 s; Medium on four, 28.4 s (about ten times Base's work). All three wrote it down word for
word; the larger ones are better on real calls (accents, crosstalk, names, a poor line).

## Another model

A `whisper….gguf` file put in the `models` folder in the settings folder
(`%APPDATA%\RecallCraft\models`) is used instead of the installed one (the first by name),
unless Small or Medium is chosen and downloaded. Another of OpenAI's English-only models
(`tiny.en`, `small.en`, `medium.en`) can be made with `convert_model`.
