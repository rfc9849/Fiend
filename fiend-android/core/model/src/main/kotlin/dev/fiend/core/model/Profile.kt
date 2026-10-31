package dev.fiend.core.model

data class Profile(
    val id: String,
    val name: String,
    val host: String,
    val port: Int,
    val secret: String,
    val templateId: String,
    val templateJson: String? = null,
    val countryCode: String? = null,
    val latencyMs: Int? = null,
    val lastConnectedAt: Long? = null,
    val tuning: Tuning = Tuning(),
    val moreServers: List<String> = emptyList(),
) {
    val authority: String get() = if (':' in host && !host.startsWith('[')) "[$host]:$port" else "$host:$port"

    val servers: List<String> get() = listOf(authority) + moreServers

    val secretHint: String get() = secret.takeLast(4)
}

data class Tuning(
    val handshakeTimeoutMs: Int = 10_000,
    val maxConnections: Int = 1_024,
    val udpIdleMs: Int = 60_000,
    val udpKeepaliveMs: Int = 20_000,
    val maxUdpFlows: Int = 64,
)

data class FingerprintTemplate(
    val id: String,
    val name: String,
    val platform: String,
    val domain: String = "",
    val templateJson: String? = null,
) {
    val label: String get() = "$name · ${domain.ifBlank { platform }}"
}
