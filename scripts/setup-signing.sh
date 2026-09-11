#!/bin/zsh
# Keep the local signing identity outside target/ and the repository so that
# rebuilding or running cargo clean cannot change the app's macOS identity.
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo '仅支持 macOS' >&2; exit 1; }
umask 077
flow_signing_dir="$HOME/Library/Application Support/Flow Insight/signing"
flow_identity_file="$flow_signing_dir/identity.sha1"
flow_certificate_name='Flow Insight Local Development'
flow_keychain="$(security default-keychain -d user | sed 's/^[[:space:]]*"//; s/"[[:space:]]*$//')"
mkdir -p "$flow_signing_dir"

if [[ -f "$flow_identity_file" ]]; then
  flow_identity="$(cat "$flow_identity_file")"
  if [[ ${#flow_identity} != 40 || "$flow_identity" == *[^0-9A-Fa-f]* ]] || ! security find-identity -p codesigning "$flow_keychain" | grep -Fq "$flow_identity"; then
    echo '原有 Flow Insight 签名证书不可用。请恢复登录钥匙串中的原证书；不会自动换证书或退回临时签名。' >&2
    exit 1
  fi
  echo "$flow_identity"
  exit 0
fi

# Recover the same identity if only the public fingerprint file was removed.
flow_existing="$(security find-certificate -a -c "$flow_certificate_name" -Z "$flow_keychain" 2>/dev/null | awk '/SHA-1 hash:/ {print $3}' || true)"
if [[ -n "$flow_existing" ]]; then
  if [[ "$flow_existing" == *$'\n'* ]]; then
    echo '发现多个 Flow Insight 本地证书，请先明确使用哪一个；不会随机切换签名。' >&2
    exit 1
  fi
  if ! security find-identity -p codesigning "$flow_keychain" | grep -Fq "$flow_existing"; then
    echo '已有 Flow Insight 证书缺少可用私钥，请恢复原签名身份。' >&2
    exit 1
  fi
  print -r -- "$flow_existing" > "$flow_identity_file"
  echo "$flow_existing"
  exit 0
fi

flow_temp="$(mktemp -d "$flow_signing_dir/setup.XXXXXX")"
trap 'rm -rf -- "$flow_temp"' EXIT
echo '创建一次性的 Flow Insight 本机开发签名，私钥保存到登录钥匙串。' >&2
/usr/bin/openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 3650 \
  -subj "/CN=$flow_certificate_name/" \
  -addext 'basicConstraints=critical,CA:FALSE' \
  -addext 'keyUsage=critical,digitalSignature' \
  -addext 'extendedKeyUsage=critical,codeSigning' \
  -keyout "$flow_temp/key.pem" -out "$flow_temp/certificate.pem" 2>"$flow_temp/openssl.log"
/usr/bin/openssl rand -hex 24 > "$flow_temp/password"
/usr/bin/openssl pkcs12 -export -inkey "$flow_temp/key.pem" \
  -in "$flow_temp/certificate.pem" -name "$flow_certificate_name" \
  -out "$flow_temp/identity.p12" -passout "file:$flow_temp/password"
# Only codesign is allowed to use this new key; do not grant every application
# access, alter existing keys, or change system certificate trust settings.
security import "$flow_temp/identity.p12" -k "$flow_keychain" -f pkcs12 \
  -P "$(cat "$flow_temp/password")" -T /usr/bin/codesign >&2
flow_identity="$(/usr/bin/openssl x509 -in "$flow_temp/certificate.pem" -noout -fingerprint -sha1 | sed 's/.*=//; s/://g')"
cp "$flow_temp/certificate.pem" "$flow_signing_dir/certificate.pem"
print -r -- "$flow_identity" > "$flow_identity_file"
echo "$flow_identity"
