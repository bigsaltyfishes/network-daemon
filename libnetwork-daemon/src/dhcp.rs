use std::net::Ipv4Addr;

#[derive(Debug, Clone)]
pub struct Lease {
    pub assigned_ip: Ipv4Addr,
    pub server_id: Ipv4Addr,
    pub subnet_mask: Ipv4Addr,
    pub router: Option<Ipv4Addr>,
    pub dns_servers: Vec<Ipv4Addr>,
    pub lease_time: u32,     // Seconds
    pub renewal_time: u32,   // T1
    pub rebinding_time: u32, // T2
}

#[derive(Default)]
pub struct LeaseBuilder {
    assigned_ip: Option<Ipv4Addr>,
    server_id: Option<Ipv4Addr>,
    subnet_mask: Option<Ipv4Addr>,
    router: Option<Ipv4Addr>,
    dns_servers: Vec<Ipv4Addr>,
    lease_time: u32,
    renewal_time: Option<u32>,
    rebinding_time: Option<u32>,
}

impl LeaseBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_assigned_ip(mut self, ip: Ipv4Addr) -> Self {
        self.assigned_ip = Some(ip);
        self
    }

    pub fn set_server_id(mut self, server_id: Ipv4Addr) -> Self {
        self.server_id = Some(server_id);
        self
    }

    pub fn set_subnet_mask(mut self, mask: Ipv4Addr) -> Self {
        self.subnet_mask = Some(mask);
        self
    }

    pub fn set_router(mut self, router: Ipv4Addr) -> Self {
        self.router = Some(router);
        self
    }

    pub fn add_dns_server(mut self, dns: Ipv4Addr) -> Self {
        self.dns_servers.push(dns);
        self
    }

    pub fn set_lease_time(mut self, lease_time: u32) -> Self {
        self.lease_time = lease_time;
        self
    }

    pub fn set_renewal_time(mut self, renewal_time: u32) -> Self {
        self.renewal_time = Some(renewal_time);
        self
    }

    pub fn set_rebinding_time(mut self, rebinding_time: u32) -> Self {
        self.rebinding_time = Some(rebinding_time);
        self
    }

    pub fn build(self) -> Result<Lease, &'static str> {
        let renewal_time = self.renewal_time.unwrap_or(self.lease_time / 2);
        let rebinding_time = self
            .rebinding_time
            .unwrap_or(((self.lease_time as u64) * 7 / 8) as u32);

        Ok(Lease {
            assigned_ip: self.assigned_ip.ok_or("Assigned IP is required")?,
            server_id: self.server_id.ok_or("Server ID is required")?,
            subnet_mask: self.subnet_mask.ok_or("Subnet mask is required")?,
            router: self.router,
            dns_servers: self.dns_servers,
            lease_time: self.lease_time,
            renewal_time,
            rebinding_time,
        })
    }
}
