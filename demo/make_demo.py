"""Build a fake multilingual meeting with macOS voices, for screenshots and demo videos.
usage: python3 demo/make_demo.py   ->  demo/nimbus-launch-sync.m4a
Every person is one voice that keeps its timbre across languages, like a real bilingual speaker.
Transcribe it with:
  hanmadi --transcribe demo/nimbus-launch-sync.m4a --title "Nimbus launch sync" --vocab Nimbus --to en \
    --speakers "Sam:en;Mina:ko,zh;Wei:zh,en;Yuki:ja,en;Claire:fr,en"
"""
import re, subprocess, sys, tempfile
from pathlib import Path

VOICE = {"Sam": "Reed", "Mina": "Flo", "Wei": "Eddy", "Yuki": "Shelley", "Claire": "Sandy"}
LOCALE = {"en": "en_US", "ko": "ko_KR", "zh": "zh_CN", "ja": "ja_JP", "fr": "fr_FR"}
LINES = [
    ("Sam", "en", "Okay, let's get started. Thanks for joining from four cities. The goal today is to lock the launch date for Nimbus."),
    ("Mina", "ko", "네, 서울은 준비가 거의 끝났습니다. 한국어 앱스토어 설명이랑 스크린샷은 어제 다 올렸어요."),
    ("Wei", "zh", "上海这边，安卓版本还有两个崩溃问题。我估计周四之前能修好。"),
    ("Mina", "zh", "周四能修好的话，我们下周一提交审核，可以吗？"),
    ("Wei", "zh", "没问题，周三晚上我先发一个测试版给大家。"),
    ("Yuki", "ja", "デザインの方は、新しいアイコンが昨日完成しました。ダークモードの色だけ、もう一度確認したいです。"),
    ("Sam", "en", "Yuki, can you share the dark mode screens in the channel after this call?"),
    ("Yuki", "en", "Sure, I'll post them in about an hour."),
    ("Mina", "ko", "그럼 출시일은 다음 달 3일로 잡을까요? 일본이랑 중국도 같은 날 여는 거죠?"),
    ("Yuki", "ja", "日本は大丈夫です。ただ、プレスリリースの翻訳に三日ほどかかります。"),
    ("Wei", "en", "China needs one more week for the app store review, so maybe we launch there on the tenth."),
    ("Claire", "fr", "Pour la France, je propose le dix aussi. Le trois, c'est encore les vacances scolaires."),
    ("Sam", "en", "Good point, Claire. So Korea and Japan on the third, China and France on the tenth. I'll update the plan."),
    ("Mina", "ko", "좋아요. 마지막으로 Nimbus 가격은 그대로 월 구천구백 원으로 가는 거 맞죠?"),
    ("Sam", "en", "Yes, same price everywhere, adjusted for local currency."),
    ("Wei", "zh", "好的，那我今天把价格配置也更新一下。"),
    ("Claire", "en", "And I'll send the French press kit to everyone by Friday."),
    ("Sam", "en", "Great. Thanks everyone, talk next Monday."),
    ("Mina", "ko", "네, 다들 수고하셨습니다. 월요일에 봬요."),
]


def voice(name, lang):
    listing = subprocess.run(["say", "-v", "?"], capture_output=True, text=True).stdout
    for line in listing.splitlines():
        m = re.match(r"^(.+?)\s+([a-z]{2}_[A-Z]{2})\s+#", line)
        if m and m.group(2) == LOCALE[lang] and re.match(rf"{VOICE[name]}( \(|$)", m.group(1)):
            return m.group(1)
    sys.exit(f"voice {VOICE[name]} for {lang} is not installed")


def main():
    out = Path(__file__).resolve().parent / "nimbus-launch-sync.m4a"
    with tempfile.TemporaryDirectory() as tmp:
        parts = []
        for i, (name, lang, text) in enumerate(LINES):
            p = Path(tmp) / f"{i:02}.aiff"
            subprocess.run(["say", "-v", voice(name, lang), "-o", str(p), text], check=True)
            parts.append(p)
        # a short pause between turns, like people waiting for each other
        n = len(parts)
        inputs = sum((["-i", str(p)] for p in parts), []) + ["-f", "lavfi", "-t", "0.6", "-i", "anullsrc=r=22050:cl=mono"]
        norm = "".join(f"[{i}:a]aresample=22050,aformat=channel_layouts=mono[v{i}];" for i in range(n))
        gaps = f"[{n}:a]asplit={n}" + "".join(f"[g{i}]" for i in range(n)) + ";"
        chain = "".join(f"[v{i}][g{i}]" for i in range(n))
        graph = f"{norm}{gaps}{chain}concat=n={2 * n}:v=0:a=1,aresample=16000[a]"
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", *inputs, "-filter_complex", graph, "-map", "[a]", "-ac", "1", "-c:a", "aac", "-b:a", "64k", str(out)], check=True)
    print(out)


if __name__ == "__main__":
    main()
