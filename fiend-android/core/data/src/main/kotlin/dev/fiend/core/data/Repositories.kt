package dev.fiend.core.data

import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.FingerprintTemplate
import dev.fiend.core.model.InstalledApp
import dev.fiend.core.model.LogLine
import dev.fiend.core.model.RuleCounts
import dev.fiend.core.model.RuleFileSpec
import dev.fiend.core.model.RuleKind
import dev.fiend.core.model.RuleSubscription
import dev.fiend.core.model.Profile
import dev.fiend.core.model.TunnelState
import dev.fiend.core.model.TunnelStats
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow

interface ProfileRepository {
    suspend fun awaitReady() { }

    val profiles: StateFlow<List<Profile>>
    val activeProfileId: StateFlow<String?>
    val templates: List<FingerprintTemplate>

    fun profile(id: String): Flow<Profile?>

    suspend fun setActive(id: String)

    suspend fun markConnected(id: String, timeEpochMs: Long) { }

    suspend fun upsert(profile: Profile): Profile

    suspend fun delete(id: String)

    suspend fun duplicate(id: String): Profile?

    suspend fun import(link: String): Result<Profile>

    suspend fun measureLatency(id: String): Int

    fun shareLink(profile: Profile): String

    fun loadTemplate(data: ByteArray): FingerprintTemplate = error("Template files are not supported")
}

interface TunnelController {
    val state: StateFlow<TunnelState>
    val stats: StateFlow<TunnelStats>
    val permissionRequired: StateFlow<Boolean>

    fun localNetworkPermissionNeeded(): Boolean = false

    fun permissionResult(granted: Boolean)

    fun connect(profileId: String)

    fun disconnect()
}

interface SettingsRepository {
    suspend fun awaitReady() { }

    val settings: StateFlow<AppSettings>

    suspend fun update(transform: (AppSettings) -> AppSettings)
}

interface RulesRepository {
    fun validate(text: String, allowIp: Boolean): Result<RuleCounts>

    fun inspectFile(path: String, kind: RuleKind): Result<RuleCounts>

    fun validateResolver(value: String): Result<Unit>

    suspend fun read(uri: String): Result<String>

    companion object {
        const val MAX_IMPORT_BYTES = 1 shl 20
    }
}

interface SubscriptionRepository {
    suspend fun refreshStale()

    suspend fun refresh(id: String): Result<Unit>

    suspend fun refreshAll(): Result<Unit>

    suspend fun add(url: String, kind: RuleKind, name: String = ""): Result<RuleSubscription>

    suspend fun edit(id: String, name: String, kind: RuleKind): Result<RuleSubscription>

    suspend fun remove(id: String)

    suspend fun ruleFiles(): List<RuleFileSpec>

    companion object {
        const val MAX_DOWNLOAD_BYTES = 32 shl 20
        const val REFRESH_INTERVAL_MS = 24L * 60 * 60 * 1000
    }
}

interface AppsRepository {
    val apps: Flow<List<InstalledApp>>
}

interface LogRepository {
    val lines: StateFlow<List<LogLine>>

    fun append(line: LogLine)

    fun clear()
}
