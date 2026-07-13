#!/usr/bin/env bash
# Add or remove one exact GitHub runner /32 SSH rule on the staging firewall.
set -euo pipefail

action="${1:?usage: scripts/digitalocean-runner-firewall.sh open|close [CIDR]}"
: "${DIGITALOCEAN_TOKEN:?DIGITALOCEAN_TOKEN is required}"
: "${DO_STAGING_FIREWALL_ID:?DO_STAGING_FIREWALL_ID is required}"

case "$action" in
  open)
    address=$(curl -4fsS --max-time 10 https://api.ipify.org)
    cidr=$(python3 -c 'import ipaddress,sys; print(ipaddress.ip_address(sys.argv[1]).compressed + "/32")' "$address")
    ;;
  close)
    cidr="${2:?close requires the exact CIDR returned by open}"
    python3 -c 'import ipaddress,sys; network=ipaddress.ip_network(sys.argv[1], strict=True); assert network.version == 4 and network.prefixlen == 32' "$cidr"
    ;;
  *)
    echo "action must be open or close" >&2
    exit 2
    ;;
esac

payload=$(jq -cn --arg cidr "$cidr" '{inbound_rules:[{protocol:"tcp",ports:"22",sources:{addresses:[$cidr]}}]}')
method=POST
if [ "$action" = close ]; then
  method=DELETE
fi
curl -fsS --retry 3 --request "$method" \
  -H 'content-type: application/json' \
  -H "authorization: Bearer $DIGITALOCEAN_TOKEN" \
  --data "$payload" \
  "https://api.digitalocean.com/v2/firewalls/$DO_STAGING_FIREWALL_ID/rules"

printf '%s\n' "$cidr"
