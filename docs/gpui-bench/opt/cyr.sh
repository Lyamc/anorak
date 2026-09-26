# Separate test instance on 127.0.0.1:8892 with Cyrillic/Greek titles in the top rows (bold-fallback check).
set -e
cd /tmp/gpuiwork
python3 - <<'PY'
import re
x=open('/tmp/gpuiwork/lodestarr-onepunch.xml',encoding='utf-8').read()
items=re.findall(r'<item>.*?</item>',x,flags=re.S)
def seeds(it):
    m=re.search(r'name="seeders" value="(\d+)"',it); return int(m.group(1)) if m else 0
top=sorted(items,key=seeds,reverse=True)[:3]
new=['Ванпанчмен / One Punch Man S03E12 [Русские субтитры] ЖЁСТКИЙ',
     'Ο Άνθρωπος με τη μία γροθιά S03E11 Ελληνικοί υπότιτλοι',
     'Смешанный: Café Ωmega naïve – ✓ S03E10']
for it,t in zip(top,new):
    x=x.replace(it, re.sub(r'<title>.*?</title>','<title>'+t+'</title>',it,count=1),1)
open('/tmp/gpuiwork/fixture-cyr.xml','w',encoding='utf-8').write(x)
print([seeds(i) for i in top])
PY
sed -e 's/18420/18421/; s/18030/18031/' mock.py > mock_cyr.py
pkill -f "mock_cyr.py" || true; pkill -f "serve-cyr/anorak" || true; sleep 0.5
SD=/tmp/gpuiwork/serve-cyr; rm -rf $SD; mkdir -p $SD
cp -r /tmp/gpuiwork/serve-wasm/assets $SD/assets; cp /tmp/gpuiwork/serve-wasm/anorak $SD/anorak; cp -r /tmp/gpuiwork/serve-wasm/gpui-dist $SD/gpui-dist
setsid nohup python3 mock_cyr.py fixture-cyr.xml /tmp/gpuiwork/rqbit-cyr.log > /tmp/gpuiwork/mock_cyr.out 2>&1 < /dev/null &
cd $SD
ANORAK_PORT=8892 ANORAK_GPUI_DIST=$SD/gpui-dist JACKETT_URL=http://127.0.0.1:18421/api/v2.0/indexers/all/results/torznab JACKETT_APIKEY=fixture RQBIT_URL=http://127.0.0.1:18031 RUST_LOG=info \
  setsid nohup $SD/anorak > /tmp/gpuiwork/anorak-cyr.out 2>&1 < /dev/null &
sleep 1.5
curl -s "http://127.0.0.1:8892/api/query?search_term=one%20punch" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['count'], [i['title'] for i in sorted(d['items'],key=lambda i:-i['seeders'])[:3]])"
