cd /tmp/gpuiwork/serve-wasm/gpui-dist
FONTS=/tmp/gpuiwork/wt-wasm/anorak-gpui/assets/fonts/ibm-plex-sans
nix-shell -p brotli zstd --run 'bash -s' <<'IN'
for f in *.wasm *.js index.html /tmp/gpuiwork/wt-wasm/anorak-gpui/assets/fonts/ibm-plex-sans/*.ttf; do
  raw=$(stat -c%s $f); gz=$(gzip -9 -c $f | wc -c); gz6=$(gzip -6 -c $f | wc -c); br=$(brotli -q 11 -c $f | wc -c); br5=$(brotli -q 5 -c $f | wc -c); zs=$(zstd -19 -c -q $f | wc -c)
  echo "$(basename $f) raw=$raw gzip9=$gz gzip6=$gz6 br11=$br br5=$br5 zstd19=$zs"
done
IN
