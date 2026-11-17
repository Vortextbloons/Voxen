import json
import re
import statistics
import unicodedata
from collections import Counter
from difflib import SequenceMatcher
from pathlib import Path

ROOT = Path(__file__).resolve().parent
OUT = ROOT.parent.parent / "runtime" / "asr"
CASES = json.loads((ROOT / "cases.json").read_text(encoding="utf-8"))["cases"]
MODELS = ["parakeet-tdt-0.6b-v3", "nemotron-en-0.6b", "nemotron-3.5-0.6b"]

ORDINALS = {
    "first": 1, "second": 2, "third": 3, "fourth": 4, "fifth": 5, "sixth": 6,
    "seventh": 7, "eighth": 8, "ninth": 9, "tenth": 10, "eleventh": 11,
    "twelfth": 12, "thirteenth": 13, "fourteenth": 14, "fifteenth": 15,
    "sixteenth": 16, "seventeenth": 17, "eighteenth": 18, "nineteenth": 19,
    "twentieth": 20, "thirtieth": 30, "fortieth": 40, "fiftieth": 50,
    "sixtieth": 60, "seventieth": 70, "eightieth": 80, "ninetieth": 90,
    "hundredth": 100, "thousandth": 1000, "millionth": 1000000,
}
SMALL = {
    "zero": 0, "one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6,
    "seven": 7, "eight": 8, "nine": 9, "ten": 10, "eleven": 11, "twelve": 12,
    "thirteen": 13, "fourteen": 14, "fifteen": 15, "sixteen": 16,
    "seventeen": 17, "eighteen": 18, "nineteen": 19, "twenty": 20,
    "thirty": 30, "forty": 40, "fifty": 50, "sixty": 60, "seventy": 70,
    "eighty": 80, "ninety": 90,
}
SCALES = {"hundred": 100, "thousand": 1000, "million": 1000000}


def fold(text):
    text = unicodedata.normalize("NFKD", text)
    text = "".join(ch for ch in text if not unicodedata.combining(ch))
    return text.replace("\u2019", "'").lower()


def words(text):
    return re.findall(r"[a-z0-9]+(?:'[a-z]+)?", fold(text))


def expanded(text):
    text = fold(text).replace("cannot", "can not").replace("can't", "can not")
    return re.sub(r"n't\b", " not", text)


def parse_numbers(text):
    raw = re.findall(r"\d+|[a-z]+(?:'[a-z]+)?", fold(text).replace(",", ""))
    tokens = raw
    values = []
    current = 0
    total = 0
    in_number = False
    for token in tokens:
        if token.isdigit():
            if in_number:
                values.append(total + current)
            total, current = int(token), 0
            values.append(total)
            total, current = 0, 0
            in_number = False
        elif token in ORDINALS:
            values.append(ORDINALS[token])
            in_number = False
        elif token in SMALL:
            current += SMALL[token]
            in_number = True
        elif token in SCALES:
            scale = SCALES[token]
            if scale == 100:
                current = max(current, 1) * scale
            else:
                total += max(current, 1) * scale
                current = 0
            in_number = True
        elif token == "and" and in_number:
            continue
        else:
            if in_number:
                values.append(total + current)
                total, current = 0, 0
                in_number = False
    if in_number:
        values.append(total + current)
    return values


def tag_value(tag):
    digits = re.sub(r"[^0-9]", "", tag)
    if digits:
        return int(digits)
    parsed = parse_numbers(tag)
    return parsed[0] if parsed else None


def number_ok(tag, hyp):
    value = tag_value(tag)
    if value is None:
        return fold(tag) in fold(hyp)
    values = parse_numbers(hyp)
    candidates = set(values)
    for first, second in zip(values, values[1:]):
        candidates.add(int(f"{first}{second}"))
    raw = re.findall(r"\d+|[a-z]+(?:'[a-z]+)?", fold(hyp))
    for first, second in zip(raw, raw[1:]):
        if first in SMALL and second in SMALL and SMALL[first] >= 20:
            candidates.add(int(f"{SMALL[first]}{SMALL[second]:02d}"))
    return value in candidates


def name_ok(name, hyp):
    gold = words(name)
    actual = words(hyp)
    return bool(gold) and any(actual[i:i + len(gold)] == gold for i in range(len(actual) - len(gold) + 1))


def negation_ok(neg, hyp):
    gold = words(expanded(neg))
    actual = words(expanded(hyp))
    return bool(gold) and any(actual[i:i + len(gold)] == gold for i in range(len(actual) - len(gold) + 1))


def edit_distance(ref, hyp):
    table = list(range(len(hyp) + 1))
    for i, r in enumerate(ref, 1):
        previous, table[0] = table[0], i
        for j, h in enumerate(hyp, 1):
            current = table[j]
            table[j] = min(table[j] + 1, table[j - 1] + 1, previous + (r != h))
            previous = current
    return table[-1]


def punctuation_f1(gold, hyp):
    marks = ". , ! ? : ;".split()
    gold_marks = Counter(ch for ch in gold if ch in marks)
    hyp_marks = Counter(ch for ch in hyp if ch in marks)
    gold_total = sum(gold_marks.values())
    hyp_total = sum(hyp_marks.values())
    if gold_total + hyp_total == 0:
        return 1.0
    shared = sum(min(gold_marks[m], hyp_marks[m]) for m in marks)
    return 2 * shared / (gold_total + hyp_total)


def case_accuracy(gold, hyp):
    a, b = words(gold), words(hyp)
    gold_tokens = re.findall(r"[A-Za-z0-9]+(?:'[A-Za-z]+)?", gold)
    hyp_tokens = re.findall(r"[A-Za-z0-9]+(?:'[A-Za-z]+)?", hyp)
    if len(gold_tokens) != len(a) or len(hyp_tokens) != len(b):
        return None
    matcher = SequenceMatcher(a=a, b=b, autojunk=False)
    matched = 0
    correct = 0
    for block in matcher.get_matching_blocks():
        for offset in range(block.size):
            matched += 1
            if gold_tokens[block.a + offset][0].isupper() == hyp_tokens[block.b + offset][0].isupper():
                correct += 1
    return correct / matched if matched else 1.0


TOKEN_RE = re.compile(r"\d+(?:[.,]\d+)*[a-z]*|[a-z]+(?:'[a-z]+)?")


def is_number_token(token):
    return bool(re.fullmatch(r"\d+(?:[.,]\d+)*[a-z]*", token)) or token in SMALL or token in SCALES or token in ORDINALS


def ordinal_suffix(value):
    if 10 <= value % 100 <= 20:
        return "th"
    return {1: "st", 2: "nd", 3: "rd"}.get(value % 10, "th")


def canonical_phrase(phrase):
    first = phrase[0].replace(",", "")
    digits = re.fullmatch(r"(\d+)([a-z]*)", first)
    ordinal = any(token in ORDINALS for token in phrase)
    if digits and len(phrase) == 1:
        value = int(digits.group(1))
        suffix = digits.group(2)
        if suffix in ("st", "nd", "rd", "th"):
            ordinal = True
            suffix = ""
        if suffix:
            return [str(value), suffix]
        return [f"{value}{ordinal_suffix(value)}" if ordinal else str(value)]
    spoken = [token for token in phrase if token != "and"]
    if (
        len(spoken) == 2
        and spoken[0] in SMALL
        and spoken[1] in SMALL
        and SMALL[spoken[0]] >= 20
    ):
        value = int(f"{SMALL[spoken[0]]}{SMALL[spoken[1]]:02d}")
    else:
        values = parse_numbers(" ".join(phrase))
        value = values[0] if values else 0
    return [f"{value}{ordinal_suffix(value)}" if ordinal else str(value)]


def canonical_tokens(text):
    raw = TOKEN_RE.findall(fold(text))
    out = []
    index = 0
    while index < len(raw):
        token = raw[index]
        if not is_number_token(token):
            out.append(token)
            index += 1
            continue
        end = index + 1
        while end < len(raw):
            if is_number_token(raw[end]):
                end += 1
            elif raw[end] == "and" and end + 1 < len(raw) and is_number_token(raw[end + 1]):
                end += 1
            else:
                break
        out.extend(canonical_phrase(raw[index:end]))
        index = end
    return out


def output_text(model, mode, case, offline_json):
    folder = "transcribe" if mode == "offline" else "transcribe-stream"
    path = OUT / model / folder / f"{Path(case['file']).stem}.json"
    if path.exists() and path.stat().st_size:
        data = json.loads(path.read_text(encoding="utf-8"))
        if "text" in data:
            return data["text"]
    fallback = "offline-output" if mode == "offline" else "stream-output"
    path = OUT / model / fallback / f"{Path(case['file']).stem}.txt"
    return path.read_text(encoding="utf-8").strip() if path.exists() else ""


def bench_summary(model, mode):
    path = OUT / model / f"{mode}.json"
    if not path.exists() or not path.stat().st_size:
        error = OUT / model / f"{mode}.err.txt"
        return {"blocked": error.read_text(encoding="utf-8").strip() if error.exists() else "no result"}
    data = json.loads(path.read_text(encoding="utf-8"))
    if "error" in data:
        return {"blocked": data["error"].get("message", str(data["error"]))}
    run = data["runs"][0]
    return {
        "loadMs": round(data["load_ms"], 1),
        "warmupMs": round(data["warmup_ms"], 1),
        "chunkMs": data.get("chunk_ms"),
        "latencyMs": {key: round(value, 1) for key, value in run["latency_ms"].items()},
        "chunkLatencyMs": {
            key: round(value, 2)
            for key, value in run.get("metrics", {}).get("chunk_latency_ms", {}).items()
        },
        "rtfx": round(run["rtfx"], 2),
        "wallSeconds": round(run["wall_seconds"], 2),
        "audioSeconds": run["audio_seconds"],
        "mismatches": run["transcript_mismatches"],
    }


def trace_summary(model):
    path = OUT / model / "stream-trace.jsonl"
    if not path.exists() or not path.stat().st_size:
        return {}
    per_case = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        entry = json.loads(line)
        per_case.setdefault(entry["input"], []).append(entry)
    rows = []
    for name, entries in per_case.items():
        first = next((entry for entry in entries if entry["text"].strip()), None)
        rows.append(
            {
                "input": name,
                "firstTextAudioS": round(first["audio_s"], 2) if first else None,
                "firstTextElapsedMs": round(first["elapsed_ms"], 1) if first else None,
                "totalElapsedMs": round(entries[-1]["elapsed_ms"], 1),
                "audioS": round(entries[-1]["audio_s"], 2),
                "finalWords": len(entries[-1]["text"].split()),
            }
        )
    firsts = [row["firstTextAudioS"] for row in rows if row["firstTextAudioS"] is not None]
    elapsed = [row["firstTextElapsedMs"] for row in rows if row["firstTextElapsedMs"] is not None]
    return {
        "cases": rows,
        "firstTextAudioS": {
            "p50": round(statistics.median(firsts), 2),
            "p95": round(sorted(firsts)[max(0, int(0.95 * len(firsts)) - 1)], 2),
            "min": min(firsts),
            "max": max(firsts),
        } if firsts else {},
        "firstTextComputeMs": {
            "p50": round(statistics.median(elapsed), 1),
            "p95": round(sorted(elapsed)[max(0, int(0.95 * len(elapsed)) - 1)], 1),
        } if elapsed else {},
    }


report = {"corpus": {"cases": len(CASES), "speech": sum(1 for c in CASES if c["transcript"]), "models": MODELS}, "models": {}}
for model in MODELS:
    run_meta = {}
    for mode in ("offline", "stream"):
        path = OUT / model / f"{mode}.run.json"
        if path.exists():
            run_meta[mode] = json.loads(path.read_text(encoding="utf-8"))
    entry = {
        "modelBytes": next((meta["modelBytes"] for meta in run_meta.values()), None),
        "peakRssMiB": max((meta["peakRssMiB"] for meta in run_meta.values()), default=None),
        "runWallMs": {mode: meta["wallMs"] for mode, meta in run_meta.items()},
        "model": next((meta["model"] for meta in run_meta.values()), None),
    }
    for mode in ("offline", "stream"):
        if mode == "stream" and model == "parakeet-tdt-0.6b-v3":
            entry["stream"] = {"blocked": "this transducer encoder is offline-only and cannot serve StreamingRecognize"}
            continue
        bench = bench_summary(model, mode)
        if "blocked" in bench and "latencyMs" not in bench:
            entry[mode] = bench
            continue
        totals = {"ref": 0, "edits": 0, "s": 0, "d": 0, "i": 0, "ref_itn": 0, "edits_itn": 0}
        rows = []
        punctuation = []
        caps = []
        names = {"matched": 0, "total": 0, "errors": []}
        numbers = {"matched": 0, "total": 0, "errors": []}
        negations = {"matched": 0, "total": 0, "errors": []}
        hallucinations = []
        for case in CASES:
            hyp = output_text(model, mode, case, mode == "offline")
            gold = case["transcript"]
            if not gold:
                if words(hyp):
                    hallucinations.append({"id": case["id"], "text": hyp})
                continue
            ref, actual = words(gold), words(hyp)
            distance = edit_distance(ref, actual)
            totals["ref"] += len(ref)
            totals["edits"] += distance
            ref_itn = canonical_tokens(gold)
            actual_itn = canonical_tokens(hyp)
            totals["ref_itn"] += len(ref_itn)
            totals["edits_itn"] += edit_distance(ref_itn, actual_itn)
            for tag in case["names"]:
                names["total"] += 1
                if name_ok(tag, hyp):
                    names["matched"] += 1
                else:
                    names["errors"].append({"id": case["id"], "tag": tag})
            for tag in case["numbers"]:
                numbers["total"] += 1
                if number_ok(tag, hyp):
                    numbers["matched"] += 1
                else:
                    numbers["errors"].append({"id": case["id"], "tag": tag})
            for tag in case["negations"]:
                negations["total"] += 1
                if negation_ok(tag, hyp):
                    negations["matched"] += 1
                else:
                    negations["errors"].append({"id": case["id"], "tag": tag})
            if case.get("punctuation_reference"):
                punctuation.append(punctuation_f1(gold, hyp))
                accuracy = case_accuracy(gold, hyp)
                if accuracy is not None:
                    caps.append(accuracy)
            rows.append(
                {
                    "id": case["id"],
                    "kind": case["kind"],
                    "refWords": len(ref),
                    "errors": distance,
                    "wer": round(distance / len(ref), 3) if ref else None,
                    "werItn": round(edit_distance(ref_itn, actual_itn) / len(ref_itn), 3) if ref_itn else None,
                    "gold": gold,
                    "hyp": hyp,
                }
            )
        entry[mode] = {
            "bench": bench,
            "wer": round(totals["edits"] / totals["ref"], 3) if totals["ref"] else None,
            "werItn": round(totals["edits_itn"] / totals["ref_itn"], 3) if totals["ref_itn"] else None,
            "refWords": totals["ref"],
            "edits": totals["edits"],
            "itnRefTokens": totals["ref_itn"],
            "itnEdits": totals["edits_itn"],
            "punctuationF1": round(statistics.mean(punctuation), 3) if punctuation else None,
            "capitalizationAccuracy": round(statistics.mean(caps), 3) if caps else None,
            "names": {**names, "accuracy": round(names["matched"] / names["total"], 3) if names["total"] else None},
            "numbers": {**numbers, "accuracy": round(numbers["matched"] / numbers["total"], 3) if numbers["total"] else None},
            "negations": {**negations, "accuracy": round(negations["matched"] / negations["total"], 3) if negations["total"] else None},
            "silenceHallucinations": hallucinations,
            "offlineVsStreamWordMismatches": (
                sum(
                    1
                    for row in rows
                    if words(row["hyp"]) != words(output_text(model, "offline", next(c for c in CASES if c["id"] == row["id"]), True))
                )
                if mode == "stream"
                else None
            ),
            "cases": rows,
        }
    entry["streamTrace"] = trace_summary(model) if "stream" in entry and "blocked" not in entry["stream"] else {}
    report["models"][model] = entry

(OUT / "report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

print(f"{'model':<26} {'mode':<8} {'WER':>6} {'ITN':>6} {'punctF1':>8} {'caps':>6} {'names':>7} {'nums':>7} {'neg':>6} {'silence':>7}")
for model, entry in report["models"].items():
    for mode in ("offline", "stream"):
        data = entry.get(mode, {})
        if "blocked" in data:
            print(f"{model:<26} {mode:<8} BLOCKED: {data['blocked'][:40]}")
            continue
        if not data:
            continue
        print(
            f"{model:<26} {mode:<8} {data['wer']:>6.3f} {data['werItn']:>6.3f} "
            f"{data['punctuationF1'] if data['punctuationF1'] is not None else '-':>8} "
            f"{data['capitalizationAccuracy'] if data['capitalizationAccuracy'] is not None else '-':>6} "
            f"{data['names']['accuracy'] if data['names']['accuracy'] is not None else '-':>7} "
            f"{data['numbers']['accuracy'] if data['numbers']['accuracy'] is not None else '-':>7} "
            f"{data['negations']['accuracy'] if data['negations']['accuracy'] is not None else '-':>6} "
            f"{len(data['silenceHallucinations']):>7}"
        )
print(f"\nwrote {OUT / 'report.json'}")
