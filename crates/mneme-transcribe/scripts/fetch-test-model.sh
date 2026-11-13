#!/bin/sh
# Downloads the smallest English Whisper model (ggml-tiny.en.bin, about 75 MB)
# into this crate's `models/` folder, which git ignores. The integration tests
# use it when it is there and skip when it is not.
#
#   crates/mneme-transcribe/scripts/fetch-test-model.sh
#
# The file comes from the ggerganov/whisper.cpp repository on Hugging Face.
# Size and SHA-256 are the values Hugging Face lists for the file (checked
# 2026-11-12); the download is verified against them before it is put in
# place.
set -eu

MODEL="ggml-tiny.en.bin"
URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${MODEL}"
SHA256="921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f"
SIZE_BYTES=77704715

models_dir="$(cd "$(dirname "$0")/.." && pwd)/models"
target="${models_dir}/${MODEL}"
partial="${target}.part"

sha256_of() {
    shasum -a 256 "$1" | cut -d ' ' -f 1
}

mkdir -p "${models_dir}"

if [ -f "${target}" ]; then
    if [ "$(sha256_of "${target}")" = "${SHA256}" ]; then
        echo "Already there and verified: ${target}"
        exit 0
    fi
    echo "Checksum of the existing ${target} is wrong; downloading it again." >&2
    rm -f "${target}"
fi

echo "Downloading ${URL} (${SIZE_BYTES} bytes)"
rm -f "${partial}"
trap 'rm -f "${partial}"' EXIT
curl --fail --location --proto '=https' --tlsv1.2 --output "${partial}" "${URL}"

actual="$(sha256_of "${partial}")"
if [ "${actual}" != "${SHA256}" ]; then
    echo "Checksum mismatch for ${MODEL}:" >&2
    echo "  expected ${SHA256}" >&2
    echo "  got      ${actual}" >&2
    exit 1
fi

mv "${partial}" "${target}"
trap - EXIT
echo "Verified SHA-256 ${actual}"
echo "Saved ${target}"
