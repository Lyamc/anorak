# Usage: s_wasm2.sh PORT [DIST_TAR] -- branch server + GPUI wasm bundle (precompressed) + fixture mocks.
PORT=${1:-8889}
TAR=${2:-/tmp/gpuiwork/dist.tar}
SD=/tmp/gpuiwork/serve-wasm
[ "$PORT" != 8889 ] && SD=/tmp/gpuiwork/serve-wasm-$PORT
pkill -f "$SD/anorak" ; sleep 0.5
if [ "$PORT" = 8889 ]; then pkill -f "/tmp/gpuiwork/mock.py"; sleep 0.3; fi
ss -ltn | grep -E ":($PORT) " && echo "PORT IN USE"
mkdir -p $SD
rm -rf $SD/assets && cp -r /tmp/gpuiwork/wt-wasm/assets $SD/assets
cp /tmp/gpuiwork/wt-wasm/target/release/anorak $SD/anorak
rm -rf $SD/gpui-dist && mkdir -p $SD/gpui-dist && tar -xf $TAR -C $SD/gpui-dist --strip-components=1
if [ "${NOPRE:-0}" != 1 ]; then
  (cd $SD/gpui-dist && nix-shell -p brotli --run 'bash /tmp/gpuiwork/wt-wasm/anorak-gpui/precompress.sh .' )
fi
if [ "$PORT" = 8889 ]; then
: > /tmp/gpuiwork/rqbit-posts.log
setsid nohup python3 /tmp/gpuiwork/mock.py /tmp/gpuiwork/lodestarr-onepunch.xml /tmp/gpuiwork/rqbit-posts.log > /tmp/gpuiwork/mock.out 2>&1 < /dev/null &
fi
cd $SD
ANORAK_PORT=$PORT ANORAK_GPUI_DIST=$SD/gpui-dist JACKETT_URL=http://127.0.0.1:18420/api/v2.0/indexers/all/results/torznab JACKETT_APIKEY=fixture RQBIT_URL=http://127.0.0.1:18030 RUST_LOG=info \
  setsid nohup $SD/anorak > /tmp/gpuiwork/anorak-wasm-$PORT.out 2>&1 < /dev/null &
sleep 1.5
ls -la $SD/gpui-dist
for f in $(ls $SD/gpui-dist | grep -vE '\.(br|gz)$'); do
  for enc in identity gzip "gzip, deflate, br"; do
    curl -s -o /dev/null -H "Accept-Encoding: $enc" -D /tmp/gpuiwork/hdr.txt -w "%{size_download}" "http://127.0.0.1:$PORT/gpui/$f" > /tmp/gpuiwork/sz.txt
    echo "$f [$enc] bytes=$(cat /tmp/gpuiwork/sz.txt) $(grep -iE '^(HTTP|content-type|content-encoding|vary|cache-control)' /tmp/gpuiwork/hdr.txt | tr -d '\r' | tr '\n' ' ')"
  done
done
curl -s -o /dev/null -w "/gpui/ %{http_code} %{content_type} %{size_download}B\n" "http://127.0.0.1:$PORT/gpui/"
curl -s -o /dev/null -w "api %{http_code} %{size_download}B\n" "http://127.0.0.1:$PORT/api/query?search_term=one%20punch"
