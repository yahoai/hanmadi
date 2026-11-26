# Voice Memos .qta embedded transcript extractor (prototype used on 2026-11-25).
# Transcript lives in an atom named com.apple.VoiceMemos.tsrp; payload is JSON starting at {"locale".
# attributedString.runs alternates [text, attrIndex, text, attrIndex, ...]; attributeTable[i].timeRange = [start, end] sec.
import sys, json
d = open(sys.argv[1], 'rb').read()
i = d.find(b'tsrp'); k = d.find(b'{"locale"', i)
j, _ = json.JSONDecoder().raw_decode(d[k:].decode('utf-8', 'ignore'))
a = j['attributedString']; tab = a['attributeTable']
for t, idx in zip(a['runs'][0::2], a['runs'][1::2]):
    st = tab[idx].get('timeRange', [None])[0]
    print(f"{st}\t{t}")
