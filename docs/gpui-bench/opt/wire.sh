# wire.sh PORT LABEL: bytes actually sent per file with a browser-like Accept-Encoding
PORT=$1; L=$2
D=$(curl -s http://127.0.0.1:$PORT/gpui/ | grep -oE 'anorak-gpui-[0-9a-f]+(_bg\.wasm|\.js)' | sort -u)
tot=0
for f in index.html $D; do
  [ $f = index.html ] && u=/gpui/ || u=/gpui/$f
  out=$(curl -s -o /dev/null -H "Accept-Encoding: gzip, deflate, br, zstd" -D /tmp/gpuiwork/h.txt -w "%{size_download}" http://127.0.0.1:$PORT$u)
  enc=$(grep -i '^content-encoding' /tmp/gpuiwork/h.txt | tr -d '\r' | awk '{print $2}')
  echo "$L $f sent=$out enc=${enc:-none}"; tot=$((tot+out))
done
echo "$L total_sent=$tot"
