"""Tidy the engine's transcript of the demo meeting for screenshots and the promo video.
usage: python3 demo/video/polish.py <app transcript.json>  ->  demo/video/transcript.json
Two of the macOS voices (Reed, Eddy) sound alike, so the engine swaps them in places, and a few
Korean words come out misheard. The text below is what the script in demo/make_demo.py actually says.
"""
import json, sys
from pathlib import Path

d = json.load(open(sys.argv[1]))
S = {s["name"]: i for i, s in enumerate(d["speakers"])}
T = d["turns"]
# (start second, speaker, text, translation): matched to the engine's turn that starts nearest
FIX = [
    (0, "Sam", "Okay, let's get started. Thanks for joining from four cities. The goal today is to lock the launch date for Nimbus.", None),
    (9, "Mina", "네, 서울은 준비가 거의 끝났습니다. 한국어 앱스토어 설명이랑 스크린샷은 어제 다 올렸어요.", "Seoul is almost ready. The Korean App Store description and screenshots went up yesterday."),
    (18, "Wei", "上海这边，安卓版本还有两个崩溃问题。我估计周四之前能修好。", "On the Shanghai side, the Android version still has two crash bugs. I expect to fix them before Thursday."),
    (25, "Mina", "周四能修好的话，我们下周一提交审核，可以吗？", "If it's fixed by Thursday, shall we submit for review next Monday?"),
    (30, "Wei", "没问题，周三晚上我先发一个测试版给大家。", "No problem. I'll send everyone a test build on Wednesday night."),
    (35, "Yuki", "デザインの方は、新しいアイコンが昨日完成しました。ダークモードの色だけ、もう一度確認したいです。", "On the design side, the new icon was finished yesterday. I'd just like to check the dark mode colors once more."),
    (44, "Sam", "Yuki, can you share the dark mode screens in the channel after this call?", None),
    (49, "Yuki", "Sure, I'll post them in about an hour.", None),
    (53, "Mina", "그럼 출시일은 다음 달 3일로 잡을까요? 일본이랑 중국도 같은 날 여는 거죠?", "Then shall we set the launch for the 3rd of next month? Japan and China open the same day, right?"),
    (61, "Yuki", "日本は大丈夫です。ただ、プレスリリースの翻訳に3日ほどかかります。", "Japan is fine. Translating the press release will take about three days, though."),
    (68, "Wei", "China needs one more week for the App Store review, so maybe we launch there on the 10th.", None),
    (74, "Claire", "Pour la France, je propose le 10 aussi. Le 3, c'est encore les vacances scolaires.", "For France, I suggest the 10th as well. The 3rd is still during the school holidays."),
    (81, "Sam", "Good point, Claire. So Korea and Japan on the 3rd, China and France on the 10th. I'll update the plan.", None),
    (89, "Mina", "좋아요. 마지막으로 Nimbus 가격은 그대로 월 9,900원으로 가는 거 맞죠?", "Great. Last thing: Nimbus stays at 9,900 won a month, right?"),
    (98, "Sam", "Yes, same price everywhere, adjusted for local currency.", None),
    (103, "Wei", "好的，那我今天把价格配置也更新一下。", "Got it. I'll update the price settings today."),
    (108, "Claire", "And I'll send the French press kit to everyone by Friday.", None),
    (112, "Sam", "Great. Thanks everyone, talk next Monday.", None),
    (116, "Mina", "네, 다들 수고하셨습니다. 월요일에 봬요.", "Thanks, everyone. See you Monday."),
]
LANGS = "en ko zh zh zh ja en en ko ja en fr en ko en zh en en ko".split()  # one per FIX line

# the engine glues Sam's "Yes..." and Wei's reply into one turn; split it where the diarization changes voice
for i, t in enumerate(T):
    if t["x"].startswith("Yes") and t["l"] != "en":
        cut = next((a for a, b, k in d["diar"] if t["t"] + 1 < a < t["e"]), (t["t"] + t["e"]) / 2)
        half = len(t.get("w", [])) // 2
        a = {**t, "e": cut, "w": t.get("w", [])[:half]}
        b = {**t, "t": cut, "w": t.get("w", [])[half:]}
        T[i:i + 1] = [a, b]
        break

out = []
for (start, who, x, tr), lang in zip(FIX, LANGS, strict=True):
    t = dict(min(T, key=lambda t: abs(t["t"] - start)))
    t.update(s=S[who], x=x, l=lang)
    t.pop("tr", None)
    if tr:
        t["tr"] = tr
    out.append(t)
d["turns"] = out
for seg in d["diar"]:  # keep the speaker map in line with the corrected turns
    m = (seg[0] + seg[1]) / 2
    seg[2] = min(out, key=lambda t: 0 if t["t"] <= m <= t["e"] else min(abs(t["t"] - m), abs(t["e"] - m)))["s"]

d.update(id="demo", audioPath="/demo/nimbus-launch-sync.m4a", terms=[], recorded="2026-11-24T10:00:00+09:00", added="2026-11-24T10:05:00+09:00")
d["source"] = {"kind": "file", "path": "/demo/nimbus-launch-sync.m4a", "recorded": d["recorded"]}
d.pop("req", None)
dst = Path(__file__).resolve().parent / "transcript.json"
json.dump(d, open(dst, "w"), ensure_ascii=False)
for t in out:
    print(round(t["t"], 1), d["speakers"][t["s"]]["name"], t["l"], t["x"][:40])
