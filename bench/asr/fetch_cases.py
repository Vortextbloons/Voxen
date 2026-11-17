import json
import urllib.parse
import urllib.request
from pathlib import Path

import numpy as np
import soundfile as sf

ROOT = Path(__file__).resolve().parent
AUDIO = ROOT / "audio"
AUDIO.mkdir(exist_ok=True)
RATE = 16000
SEED = 20261116


def get_json(url):
    with urllib.request.urlopen(url) as response:
        return json.load(response)


def write(relative, audio, rate=RATE):
    path = AUDIO / relative
    sf.write(path, audio, rate, subtype="PCM_16")
    return relative


def fetch_row(dataset, config, split, offset):
    url = (
        "https://datasets-server.huggingface.co/rows"
        f"?dataset={urllib.parse.quote(dataset)}&config={config}"
        f"&split={split}&offset={offset}&length=1"
    )
    return get_json(url)["rows"][0]["row"]


def fetch_audio(url):
    with urllib.request.urlopen(url) as response:
        return response.read()


def read_bytes(data):
    import io

    audio, rate = sf.read(io.BytesIO(data), dtype="float32")
    if audio.ndim > 1:
        audio = audio.mean(axis=1)
    return audio, rate


fleurs = [
    (20, [], [], ["not"], "satellite phone negation"),
    (42, ["1,400", "2010", "8", "2008"], ["Australia"], [], "numbers and country name"),
    (91, ["fourth", "fifth", "sixth"], ["Newt Gingrich", "Rick Perry", "Michele Bachmann", "Texas"], [], "politician names"),
    (112, ["19,500", "14"], [], [], "large number and unit"),
    (141, ["100m", "200m", "three"], ["Commonwealth Games"], [], "distances and relays"),
    (179, [], ["Rockhampton"], ["do not"], "crocodile negation"),
    (208, ["1,000th", "2000"], ["David Klöcker Ehrenstrahl", "Guinness Book of World Records"], [], "award name and years"),
    (217, [], ["Filipino", "Hong Kong", "Manila", "Philippines"], [], "place and nationality names"),
    (238, [], [], ["cannot"], "icy road negation"),
    (29, [], [], [], "AI technical sentence"),
    (295, [], ["Miller", "Mike"], ["wasn't"], "quoted speech and negation"),
    (75, ["100"], [], [], "levee measurement"),
]

cases = []
for offset, numbers, names, negations, note in fleurs:
    row = fetch_row("google/fleurs", "en_us", "validation", offset)
    audio, rate = read_bytes(fetch_audio(row["audio"][0]["src"]))
    assert rate == RATE, rate
    relative = write(f"fleurs-{row['id']}.wav", audio)
    cases.append(
        {
            "id": f"fleurs-{row['id']}",
            "file": relative,
            "source": "google/fleurs en_us validation (CC-BY-4.0)",
            "kind": "real",
            "transcript": row["raw_transcription"],
            "numbers": numbers,
            "names": names,
            "negations": negations,
            "note": note,
            "punctuation_reference": True,
        }
    )

libri = [
    ("2277-149896-0008", "negation and long clause"),
    ("2277-149896-0026", "street scene"),
]
rows = get_json(
    "https://datasets-server.huggingface.co/rows"
    "?dataset=openslr%2Flibrispeech_asr&config=clean&split=validation&offset=0&length=30"
)["rows"]
by_id = {row["row"]["id"]: row["row"] for row in rows}
for clip, note in libri:
    row = by_id[clip]
    audio, rate = read_bytes(fetch_audio(row["audio"][0]["src"]))
    assert rate == RATE, rate
    relative = write(f"librispeech-{clip}.wav", audio)
    cases.append(
        {
            "id": f"librispeech-{clip}",
            "file": relative,
            "source": "openslr/librispeech_asr clean validation (CC-BY-4.0)",
            "kind": "real",
            "transcript": row["text"].capitalize().rstrip(".") + ".",
            "numbers": [],
            "names": [],
            "negations": ["nothing"] if "NOTHING" in row["text"] else [],
            "note": note,
            "punctuation_reference": False,
        }
    )

jfk, jfk_rate = sf.read(ROOT.parent.parent / "runtime" / "jfk.wav", dtype="float32")
assert jfk_rate == RATE
cases.append(
    {
        "id": "jfk",
        "file": write("jfk.wav", jfk),
        "source": "runtime/jfk.wav (public domain speech)",
        "kind": "real",
        "transcript": "And so, my fellow Americans, ask not what your country can do for you, ask what you can do for your country.",
        "numbers": [],
        "names": ["Americans"],
        "negations": ["not"],
        "note": "public-domain address excerpt",
        "punctuation_reference": True,
    }
)

rng = np.random.default_rng(SEED)


def add_noise(audio, snr_db):
    noise = rng.standard_normal(audio.shape[0]).astype(np.float32)
    speech_rms = float(np.sqrt(np.mean(audio**2)))
    noise_rms = float(np.sqrt(np.mean(noise**2)))
    target = speech_rms / (10 ** (snr_db / 20))
    return np.clip(audio + noise * (target / noise_rms), -1.0, 1.0)


noise_specs = [
    (next(case for case in cases if "satellite" in case["note"]), 12),
    (next(case for case in cases if "crocodile" in case["note"]), 5),
]
for source, snr in noise_specs:
    base_id = source["id"]
    audio, _ = sf.read(AUDIO / source["file"], dtype="float32")
    relative = write(f"{base_id}-noise-{snr}db.wav", add_noise(audio, snr))
    variant = dict(source)
    variant.update(
        {
            "id": f"{base_id}-noise-{snr}db",
            "file": relative,
            "kind": "real+noise",
            "note": f"{source['note']}; white noise mixed at {snr} dB SNR",
        }
    )
    cases.append(variant)

silence = np.zeros(RATE * 5, dtype=np.float32)
cases.append(
    {
        "id": "silence-digital",
        "file": write("silence-digital.wav", silence),
        "source": "generated silence",
        "kind": "silence",
        "transcript": "",
        "numbers": [],
        "names": [],
        "negations": [],
        "note": "digital silence, hallucination probe",
        "punctuation_reference": False,
    }
)
floor = (rng.standard_normal(RATE * 5).astype(np.float32) * 0.001)
cases.append(
    {
        "id": "silence-noise-floor",
        "file": write("silence-noise-floor.wav", floor),
        "source": "generated low-level noise",
        "kind": "silence",
        "transcript": "",
        "numbers": [],
        "names": [],
        "negations": [],
        "note": "white noise at -60 dBFS, hallucination probe",
        "punctuation_reference": False,
    }
)

(ROOT / "cases.json").write_text(
    json.dumps({"cases": cases}, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
)
print(f"wrote {len(cases)} cases to {ROOT / 'cases.json'}")
