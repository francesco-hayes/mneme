#!/bin/sh
# Makes a few minutes of synthetic meeting speech for `mneme-bench`:
#
#   crates/mneme-transcribe/scripts/make-bench-audio.sh
#
# writes crates/mneme-transcribe/bench-audio/meeting-sample.wav (16 kHz, mono,
# 16-bit) and meeting-sample.txt (what was said, one line per turn). The
# folder is ignored by git.
#
# The speech comes from the macOS `say` command: six voices taking turns,
# with pauses of different lengths between the turns, a few one-word answers,
# two long silences, and one long turn without a break. It needs `say`,
# `afconvert` and `python3`, which the Xcode Command Line Tools provide. A
# voice that is not installed is replaced by the system voice.
#
# This is a stand-in for a meeting, good for timing a model. It is not real
# meeting audio: the voices are clean and even, nobody talks over anybody,
# and the pauses are digital silence. A real recording will differ, both in
# how well it is transcribed and in where the chunker cuts it. The output
# also depends on the voices of the macOS version it runs on.
set -eu

out_dir="$(cd "$(dirname "$0")/.." && pwd)/bench-audio"
target="${out_dir}/meeting-sample.wav"
script_text="${out_dir}/meeting-sample.txt"

for tool in say afconvert python3; do
    if ! command -v "${tool}" >/dev/null 2>&1; then
        echo "make-bench-audio.sh needs \`${tool}\`, which was not found." >&2
        exit 1
    fi
done

mkdir -p "${out_dir}"
work="$(mktemp -d "${out_dir}/work.XXXXXX")"
trap 'rm -rf "${work}"' EXIT

installed_voices="$(say -v '?')"

# One turn per line: voice | silence after the turn, in milliseconds | text.
turns() {
    cat <<'EOF'
Samantha|900|Good morning everyone. Thanks for joining. Let's get started with the weekly planning meeting.
Samantha|1400|We have three things on the agenda today. The release schedule, the support backlog, and the budget for next quarter.
Daniel|300|Sounds good.
Karen|1100|Before we start, can everybody see my screen?
Moira|250|Yes.
Rishi|200|Yes, I can see it.
Tessa|1800|It is a little small, but I can read it.
Samantha|1200|Great. Daniel, would you like to begin with the release schedule?
Daniel|2200|Sure. So the current plan is to ship version four point two on the fourteenth of March. The feature work is finished, and what remains is testing and documentation. We found eleven bugs in the last test cycle. Seven of them are fixed, three are in review, and one is still open. The open one is the crash on import when the file is larger than two gigabytes. I think we need another week for that, because the fix touches the storage layer and I do not want to rush it. If we slip, we slip by one week, to the twenty first of March. Marketing already knows about that risk, and they told me a one week delay is acceptable as long as we tell them before Friday. So my proposal is that we decide on Thursday, after the review of the storage fix, whether we keep the fourteenth or move to the twenty first.
Karen|600|That makes sense to me.
Moira|1500|I have a question. Does the one week delay affect the training sessions we planned for the sales team?
Daniel|1000|It might. The first session is on the eighteenth, and they would be training on a version that is not released yet.
Moira|400|Okay.
Moira|2500|Then I would rather move the training by one week as well, and tell them today.
Samantha|800|Agreed. Moira, can you send that message?
Moira|5000|Yes, I will do it right after this call.
Samantha|1300|Let's move on to the support backlog. Rishi, how does it look?
Rishi|1700|Not great, to be honest. We have one hundred and forty two open tickets, which is thirty more than last week. Most of the new ones are about the login change. People do not find the new sign in button, and they write to us instead.
Tessa|700|Could we add a short note on the login page?
Rishi|350|We could.
Karen|1600|I can do that today. It is a small change, maybe twenty minutes of work, and it does not need a release.
Rishi|1200|That would help a lot. The second problem is response time. Our target is four hours, and at the moment we are at nine.
Samantha|900|What do you need to get back to four?
Rishi|2000|One more person for two weeks, or we stop answering the low priority tickets for a while. I would prefer the first option.
Tessa|1300|I can lend you Jordan from my team. He knows the product well, and his project is waiting for a customer answer anyway.
Rishi|300|Perfect, thank you.
Samantha|12000|Good. Let me share the budget sheet. Give me a moment to find it.
Samantha|1500|Okay, here it is. For next quarter we asked for two hundred and sixty thousand, and we got two hundred and thirty.
Daniel|450|So we are thirty thousand short.
Samantha|1100|Exactly. We have to cut something, and I would like to hear your ideas.
Karen|1800|The largest item is the cloud bill. Last quarter it was eighty five thousand. I believe we can save ten to fifteen percent by turning off the test environments at night and on weekends. Nobody uses them then.
Tessa|900|That is around ten thousand. What else?
Moira|1400|We planned two conferences. We could go to one, and send two people where we wanted to send four. That saves about eight thousand.
Daniel|600|Which one would we skip?
Moira|1700|The one in October. It is expensive, and last year we did not get a single lead from it.
Rishi|800|I am fine with that.
Tessa|2100|For the remaining twelve thousand, I suggest we delay the new laptops by one quarter. The old ones are slow, but they work.
Karen|500|Hmm. I am not sure about that.
Karen|1600|Two of my developers lose about half an hour every day waiting for builds. That costs more than a laptop.
Samantha|1000|Then let's replace those two now, and delay the rest.
Karen|300|Okay.
Tessa|1400|Fine with me.
Samantha|1800|Good. So, to summarize. Daniel decides on Thursday whether the release stays on the fourteenth. Moira moves the sales training and informs the team today. Karen adds the note to the login page and looks into switching off the test environments. Tessa lends Jordan to support for two weeks. And I update the budget sheet and send it around by Wednesday.
Samantha|900|Did I forget anything?
Daniel|400|No.
Rishi|1200|No, that was everything from my side.
Moira|1500|One small thing. Next Monday is a public holiday in Ireland, so I will not be here.
Samantha|700|Thanks for the reminder. Then we meet on Tuesday instead, same time.
Tessa|300|Works for me.
Karen|250|Yes.
Daniel|1000|Tuesday is fine.
Samantha|800|Great. Thank you all, and have a good week.
Rishi|300|Thanks, bye.
Moira|0|Bye everyone.
EOF
}

: > "${work}/pieces.txt"
: > "${script_text}"
number=0
turns | while IFS='|' read -r voice pause text; do
    number=$((number + 1))
    piece="${work}/$(printf '%03d' "${number}")"
    # Voice names in the list are followed by spaces, or by " (" for voices
    # that exist in several variants.
    if printf '%s\n' "${installed_voices}" | grep -q -E "^${voice}( |\$)"; then
        say -v "${voice}" -o "${piece}.aiff" "${text}" </dev/null
    else
        echo "Voice ${voice} is not installed; using the system voice for turn ${number}." >&2
        say -o "${piece}.aiff" "${text}" </dev/null
    fi
    afconvert -f WAVE -d LEI16@16000 -c 1 "${piece}.aiff" "${piece}.wav" </dev/null
    rm -f "${piece}.aiff"
    printf '%s|%s\n' "${piece}.wav" "${pause}" >> "${work}/pieces.txt"
    printf '%s: %s\n' "${voice}" "${text}" >> "${script_text}"
done

# Joins the turns, with digital silence of the given length after each one.
python3 - "${work}/pieces.txt" "${target}.part" <<'EOF'
import sys
import wave

pieces_file, target = sys.argv[1], sys.argv[2]
RATE = 16000

with wave.open(target, "wb") as out:
    out.setnchannels(1)
    out.setsampwidth(2)
    out.setframerate(RATE)
    for line in open(pieces_file):
        path, pause_ms = line.rstrip("\n").split("|")
        with wave.open(path, "rb") as piece:
            shape = (piece.getnchannels(), piece.getsampwidth(), piece.getframerate())
            if shape != (1, 2, RATE):
                sys.exit(f"{path}: expected 16 kHz mono 16-bit, got {shape}")
            out.writeframes(piece.readframes(piece.getnframes()))
        out.writeframes(b"\x00\x00" * (RATE * int(pause_ms) // 1000))

with wave.open(target, "rb") as result:
    seconds = result.getnframes() / RATE
print(f"{seconds:.2f} s of audio")
EOF

mv "${target}.part" "${target}"
echo "Saved ${target}"
echo "Saved ${script_text}"
shasum -a 256 "${target}"
