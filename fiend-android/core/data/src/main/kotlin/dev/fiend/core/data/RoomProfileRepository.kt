package dev.fiend.core.data

import androidx.datastore.preferences.core.edit
import dev.fiend.core.data.storage.PreferencesStore
import dev.fiend.core.data.storage.ProfileDatabase
import dev.fiend.core.data.storage.ProfileEntity
import dev.fiend.core.data.storage.SecretCipher
import dev.fiend.core.engine.bindings.bundledTemplates
import dev.fiend.core.engine.bindings.makeShareLink
import dev.fiend.core.engine.bindings.measureLatency as nativeLatency
import dev.fiend.core.engine.bindings.parseShareLink
import dev.fiend.core.engine.bindings.validateProfile
import dev.fiend.core.engine.engineConfig
import dev.fiend.core.model.FingerprintTemplate
import dev.fiend.core.model.LogLevel
import dev.fiend.core.model.LogLine
import dev.fiend.core.model.Profile
import dev.fiend.core.model.Tuning
import java.net.URI
import java.security.MessageDigest
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONObject

@Singleton
internal class RoomProfileRepository @Inject constructor(
    database: ProfileDatabase,
    private val cipher: SecretCipher,
    private val store: PreferencesStore,
    private val logs: LogRepository,
    @ApplicationScope scope: CoroutineScope,
) : ProfileRepository {
    private val dao = database.profiles()
    private val mutableProfiles = MutableStateFlow<List<Profile>>(emptyList())
    private val active = MutableStateFlow<String?>(null)
    override val profiles = mutableProfiles.asStateFlow()
    override val activeProfileId = active.asStateFlow()
    private val bundled = bundledTemplates()
    override val templates: List<FingerprintTemplate> get() = bundled.map { FingerprintTemplate(it.id, it.name, it.platform) } +
        profiles.value.filter { it.templateId.startsWith("custom-") }.distinctBy { it.templateId }.map { FingerprintTemplate(it.templateId, "Imported template", "TLS") }
    private val profilesReady = CompletableDeferred<Unit>()
    private val activeReady = CompletableDeferred<Unit>()
    private val writes = Mutex()

    init {
        scope.launch(Dispatchers.IO) {
            try {
                dao.observe().collect { records ->
                    mutableProfiles.value = records.mapNotNull { record ->
                        runCatching { record.model() }.onFailure { logs.append(LogLine(System.currentTimeMillis(), LogLevel.Error, "storage", "Cannot decrypt a saved profile")) }.getOrNull()
                    }
                    profilesReady.complete(Unit)
                }
            } catch (error: Exception) { profilesReady.completeExceptionally(error); logs.append(LogLine(System.currentTimeMillis(), LogLevel.Error, "storage", "Cannot read saved profiles")) }
        }
        scope.launch {
            try { store.data.data.collect { active.value = it[PreferencesStore.ACTIVE_PROFILE]; activeReady.complete(Unit) } }
            catch (error: Exception) { activeReady.completeExceptionally(error) }
        }
    }

    override suspend fun awaitReady() { profilesReady.await(); activeReady.await() }
    override fun profile(id: String) = profiles.map { it.firstOrNull { profile -> profile.id == id } }.distinctUntilChanged()

    override suspend fun setActive(id: String) = writes.withLock {
        require(dao.get(id) != null) { "The selected profile is not available" }
        store.data.edit { it[PreferencesStore.ACTIVE_PROFILE] = id }
        active.value = id
    }

    override suspend fun upsert(profile: Profile): Profile = withContext(Dispatchers.IO) {
        awaitReady()
        writes.withLock {
            val template = bundled.firstOrNull { it.id == profile.templateId }?.templateJson ?: profile.templateJson
            val stored = profile.copy(id = profile.id.ifEmpty { UUID.randomUUID().toString() }, templateJson = template)
            validateProfile(stored.engineConfig())
            require(stored.port in 1..65535) { "Port must be between 1 and 65535" }
            require(stored.tuning.handshakeTimeoutMs > 0 && stored.tuning.maxConnections > 0 && stored.tuning.udpIdleMs > stored.tuning.udpKeepaliveMs && stored.tuning.udpKeepaliveMs > 0 && stored.tuning.maxUdpFlows in 1..4096) { "Connection settings are not valid" }
            dao.upsert(stored.entity())
            if (active.value == null) {
                store.data.edit { it[PreferencesStore.ACTIVE_PROFILE] = stored.id }
                active.value = stored.id
            }
            stored
        }
    }

    override suspend fun delete(id: String) = writes.withLock {
        dao.delete(id)
        if (active.value == id) {
            val next = dao.all().firstOrNull()?.id
            store.data.edit { if (next == null) it.remove(PreferencesStore.ACTIVE_PROFILE) else it[PreferencesStore.ACTIVE_PROFILE] = next }
            active.value = next
        }
    }

    override suspend fun markConnected(id: String, timeEpochMs: Long) { dao.connected(id, timeEpochMs) }

    override suspend fun duplicate(id: String): Profile? {
        val original = withContext(Dispatchers.IO) { dao.get(id)?.model() } ?: return null
        return upsert(original.copy(id = "", name = "${original.name} (copy)", lastConnectedAt = null))
    }

    override suspend fun import(link: String): Result<Profile> = withContext(Dispatchers.IO) {
        runCatching {
            val native = parseShareLink(link.trim())
            val address = URI("tcp://${native.server}")
            require(address.userInfo == null && address.host != null && address.port in 1..65535) { "Server address is not valid" }
            val capture = JSONObject(native.templateJson)
            val templateId = bundled.firstOrNull {
                val value = JSONObject(it.templateJson)
                value.getString("raw") == capture.getString("raw") && value.optInt("record_max", 16384) == capture.optInt("record_max", 16384)
            }?.id ?: "custom-" + MessageDigest.getInstance("SHA-256").digest(native.templateJson.toByteArray()).joinToString("") { "%02x".format(it) }
            upsert(Profile("", native.name, address.host.removeSurrounding("[", "]"), address.port, native.secret, templateId, templateJson = native.templateJson))
        }
    }

    override suspend fun measureLatency(id: String): Int = withContext(Dispatchers.IO) {
        val profile = dao.get(id)?.model() ?: error("The selected profile is not available")
        val latency = nativeLatency(profile.engineConfig()).toInt()
        dao.latency(id, latency)
        latency
    }

    override fun shareLink(profile: Profile): String = makeShareLink(profile.engineConfig())

    private fun Profile.entity() = ProfileEntity(id, name, host, port, cipher.encrypt(id, secret), templateId, templateJson,
        countryCode, latencyMs, lastConnectedAt, tuning.handshakeTimeoutMs, tuning.maxConnections, tuning.udpIdleMs, tuning.udpKeepaliveMs, tuning.maxUdpFlows)
    private fun ProfileEntity.model() = Profile(id, name, host, port, cipher.decrypt(id, encryptedSecret), templateId,
        templateJson = templateJson, countryCode = countryCode, latencyMs = latencyMs, lastConnectedAt = lastConnectedAt,
        tuning = Tuning(handshakeTimeoutMs, maxConnections, udpIdleMs, udpKeepaliveMs, maxUdpFlows))
}
