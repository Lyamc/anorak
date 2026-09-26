# Usage: s_wasm.sh PORT  -- branch server + GPUI wasm bundle + fixture mocks.
PORT=${1:-8889}
SD=/tmp/gpuiwork/serve-wasm
pkill -f "/tmp/gpuiwork/mock.py" ; pkill -f "$SD/anorak" ; sleep 0.5
ss -ltn | grep -E ":($PORT|18420|18030) " && echo "PORT IN USE"
mkdir -p $SD
rm -rf $SD/assets && cp -r /tmp/gpuiwork/wt-wasm/assets $SD/assets
cp /tmp/gpuiwork/wt-wasm/target/release/anorak $SD/anorak
if [ -f /tmp/gpuiwork/dist.tar ]; then rm -rf $SD/gpui-dist && mkdir -p $SD/gpui-dist && tar -xf /tmp/gpuiwork/dist.tar -C $SD/gpui-dist --strip-components=1; fi
: > /tmp/gpuiwork/rqbit-posts.log
setsid nohup python3 /tmp/gpuiwork/mock.py /tmp/gpuiwork/lodestarr-onepunch.xml /tmp/gpuiwork/rqbit-posts.log > /tmp/gpuiwork/mock.out 2>&1 < /dev/null &
cd $SD
ANORAK_PORT=$PORT ANORAK_GPUI_DIST=$SD/gpui-dist JACKETT_URL=http://127.0.0.1:18420/api/v2.0/indexers/all/results/torznab JACKETT_APIKEY=fixture RQBIT_URL=http://127.0.0.1:18030 RUST_LOG=info \
  setsid nohup $SD/anorak > /tmp/gpuiwork/anorak-wasm.out 2>&1 < /dev/null &
sleep 1.5
ls -la $SD/gpui-dist
tail -2 /tmp/gpuiwork/anorak-wasm.out
for f in $(ls $SD/gpui-dist); do curl -s -o /dev/null -D - "http://127.0.0.1:$PORT/gpui/$f" | grep -iE "^(HTTP|content-type|content-length|content-encoding)" | tr -d '\r' | tr '\n' ' '; echo " <- $f"; done
curl -s -o /dev/null -w "/gpui/ %{http_code} %{content_type} %{size_download}B\n" "http://127.0.0.1:$PORT/gpui/"
curl -s -o /dev/null -w "/gpui %{http_code} %{content_type} %{size_download}B\n" "http://127.0.0.1:$PORT/gpui"
curl -s -o /dev/null -H 'Accept-Encoding: gzip, br' -w "gz-accept /gpui/ %{http_code} %{size_download}B\n" "http://127.0.0.1:$PORT/gpui/"
curl -s -o /tmp/gpuiwork/api.json -w "api %{http_code} %{size_download}B %{time_total}s\n" "http://127.0.0.1:$PORT/api/query?search_term=one%20punch"
curl -s -o /dev/null -w "web / %{http_code} %{size_download}B\n" "http://127.0.0.1:$PORT/"
