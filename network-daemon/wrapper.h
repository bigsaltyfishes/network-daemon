/*
 * Minimal libifconfig bindings for wutil-rs
 * Only include necessary headers for network interface management
 */

#include <sys/socket.h>
#include <netlink/netlink.h>
#include <netlink/netlink_route.h>
#include <netlink/route/common.h>
#include <sys/sockio.h>

#include <sys/ioctl.h>
#include <net/if.h>
#include <net80211/_ieee80211.h>
#include <net80211/ieee80211_ioctl.h>
#include <lib80211/lib80211_regdomain.h>
#include <lib80211/lib80211_ioctl.h>
#include <netinet/in.h>
#include <netinet6/in6_var.h>
#include <netinet6/nd6.h>