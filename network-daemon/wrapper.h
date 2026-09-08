/* Kernel-facing FreeBSD network interface definitions used by the daemon. */

#include <sys/socket.h>
#include <netlink/netlink.h>
#include <netlink/netlink_route.h>
#include <netlink/route/common.h>
#include <sys/sockio.h>

#include <sys/ioctl.h>
#include <net/if.h>
#include <net/if_bridgevar.h>
#include <net/if_lagg.h>
#include <net/if_vlan_var.h>
#include <net80211/_ieee80211.h>
#include <net80211/ieee80211_ioctl.h>
#include <netinet/in.h>
#include <netinet6/in6_var.h>
#include <netinet6/nd6.h>

/* Keep ioctl request values tied to the target kernel headers. */
enum {
    ND_SIOCGIFFLAGS = SIOCGIFFLAGS,
    ND_SIOCSIFFLAGS = SIOCSIFFLAGS,
    ND_SIOCIFCREATE2 = SIOCIFCREATE2,
    ND_SIOCGIFINFO_IN6 = SIOCGIFINFO_IN6,
    ND_SIOCSIFINFO_IN6 = SIOCSIFINFO_IN6,
    ND_SIOCIFDESTROY = SIOCIFDESTROY,
    ND_SIOCSDRVSPEC = SIOCSDRVSPEC,
    ND_SIOCSETVLAN = SIOCSETVLAN,
    ND_SIOCSLAGG = SIOCSLAGG,
    ND_SIOCSLAGGPORT = SIOCSLAGGPORT,
    ND_BRDGADD = BRDGADD,
    ND_LAGG_PROTO_NONE = LAGG_PROTO_NONE,
    ND_LAGG_PROTO_ROUNDROBIN = LAGG_PROTO_ROUNDROBIN,
    ND_LAGG_PROTO_FAILOVER = LAGG_PROTO_FAILOVER,
    ND_LAGG_PROTO_LOADBALANCE = LAGG_PROTO_LOADBALANCE,
    ND_LAGG_PROTO_LACP = LAGG_PROTO_LACP,
    ND_LAGG_PROTO_BROADCAST = LAGG_PROTO_BROADCAST
};
