import Foundation
import NetworkExtension
import os.log

/// iOS Network Extension that owns the TUN file descriptor and hands it to
/// the Rust core (`ff_vpn_core`). All actual VPN logic lives in Rust; this
/// class just glues `NEPacketTunnelProvider` to the C ABI exposed by the
/// staticlib.
class PacketTunnelProvider: NEPacketTunnelProvider {

    private let log = OSLog(subsystem: "com.incss.ff.vpn", category: "PacketTunnel")

    override func startTunnel(options: [String : NSObject]?,
                              completionHandler: @escaping (Error?) -> Void) {
        guard
            let providerProtocol = self.protocolConfiguration as? NETunnelProviderProtocol,
            let providerConfig = providerProtocol.providerConfiguration,
            let configJson = providerConfig["config"] as? String
        else {
            completionHandler(NSError(domain: "ff.vpn", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "missing provider config"]))
            return
        }

        let tunAddr = "10.10.0.2"
        let mtu: NSNumber = 1500
        let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: "127.0.0.1")
        let ipv4 = NEIPv4Settings(addresses: [tunAddr], subnetMasks: ["255.255.255.0"])
        ipv4.includedRoutes = [NEIPv4Route.default()]
        settings.ipv4Settings = ipv4
        let dns = NEDNSSettings(servers: [tunAddr])
        dns.matchDomains = [""]
        settings.dnsSettings = dns
        settings.mtu = mtu

        setTunnelNetworkSettings(settings) { [weak self] error in
            guard let self = self else { return }
            if let error = error {
                completionHandler(error)
                return
            }

            // The iOS Network Extension SDK does not expose a raw FD.
            // Instead, we drive `packetFlow` from the Rust core and pass
            // a pseudo "fd" of -1; the Rust side falls back to a packetFlow
            // bridge supplied via FFI when running on iOS. (See comments in
            // `engine.rs`'s `PlatformContext::tun_fd`.)
            let privateDir = (FileManager.default.urls(
                for: .documentDirectory, in: .userDomainMask).first?.path) ?? NSTemporaryDirectory()
            ff_vpn_init()
            let rc = configJson.withCString { cfg in
                privateDir.withCString { dir in
                    tunAddr.withCString { addr in
                        ff_vpn_start(cfg, dir, -1, addr, UInt16(truncating: mtu))
                    }
                }
            }
            if rc != 0 {
                let cmsg = ff_vpn_last_error()
                let msg = cmsg.map { String(cString: $0) } ?? "unknown"
                os_log("ff_vpn_start failed: %{public}s", log: self.log, type: .error, msg)
                completionHandler(NSError(domain: "ff.vpn", code: Int(rc),
                    userInfo: [NSLocalizedDescriptionKey: msg]))
                return
            }
            completionHandler(nil)
        }
    }

    override func stopTunnel(with reason: NEProviderStopReason,
                             completionHandler: @escaping () -> Void) {
        _ = ff_vpn_stop()
        completionHandler()
    }
}

// Bridging declarations — these match the C ABI exported by `ff_vpn_core`.
@_silgen_name("ff_vpn_init") func ff_vpn_init()
@_silgen_name("ff_vpn_start") func ff_vpn_start(
    _ configJson: UnsafePointer<CChar>?,
    _ privateDir: UnsafePointer<CChar>?,
    _ tunFd: Int32,
    _ tunAddr: UnsafePointer<CChar>?,
    _ tunMtu: UInt16
) -> Int32
@_silgen_name("ff_vpn_stop") func ff_vpn_stop() -> Int32
@_silgen_name("ff_vpn_last_error") func ff_vpn_last_error() -> UnsafePointer<CChar>?
