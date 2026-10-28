package dev.fiend.core.data

import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.FingerprintTemplate
import dev.fiend.core.model.InstalledApp
import dev.fiend.core.model.LogLine
import dev.fiend.core.model.RuleCounts
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

    fun validateResolver(value: String): Result<Unit>

    suspend fun read(uri: String): Result<String>

    companion object {
        const val MAX_IMPORT_BYTES = 1 shl 20
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
