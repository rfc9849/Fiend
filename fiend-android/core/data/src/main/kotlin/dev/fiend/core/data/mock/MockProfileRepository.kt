package dev.fiend.core.data.mock

import dev.fiend.core.data.ProfileRepository
import dev.fiend.core.model.Profile
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton
import kotlin.random.Random
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update

@Singleton
class MockProfileRepository @Inject constructor() : ProfileRepository {
    private val state = MutableStateFlow(MockData.profiles(System.currentTimeMillis()))
    private val active = MutableStateFlow<String?>("ams")

    override val profiles: StateFlow<List<Profile>> = state.asStateFlow()
    override val activeProfileId: StateFlow<String?> = active.asStateFlow()
    override val templates = MockData.templates

    override fun profile(id: String): Flow<Profile?> =
        state.map { list -> list.firstOrNull { it.id == id } }.distinctUntilChanged()

    override suspend fun setActive(id: String) {
        active.value = id
    }

    override suspend fun upsert(profile: Profile): Profile {
        val stored = if (profile.id.isEmpty()) profile.copy(id = UUID.randomUUID().toString()) else profile
        state.update { list ->
            if (list.any { it.id == stored.id }) {
                list.map { if (it.id == stored.id) stored else it }
            } else {
                list + stored
            }
        }
        if (active.value == null) active.value = stored.id
        return stored
    }

    override suspend fun delete(id: String) {
        state.update { list -> list.filterNot { it.id == id } }
        if (active.value == id) active.value = state.value.firstOrNull()?.id
    }

    override suspend fun duplicate(id: String): Profile? {
        val source = state.value.firstOrNull { it.id == id } ?: return null
        return upsert(source.copy(id = "", name = "${source.name} (copy)", lastConnectedAt = null))
    }

    override suspend fun import(link: String): Result<Profile> {
        delay(450)
        val trimmed = link.trim()
        if (!trimmed.startsWith("fiend://")) {
            return Result.failure(IllegalArgumentException("Not a fiend link"))
        }
        val host = trimmed.substringAfter('@', "").substringBefore('?').substringBefore('#')
            .substringBefore(':').ifEmpty { "imported.example.net" }
        val name = trimmed.substringAfter('#', "").ifEmpty { host.substringBefore('.').replaceFirstChar(Char::uppercase) }
        val secret = buildString { repeat(64) { append("0123456789abcdef"[Random.nextInt(16)]) } }
        return Result.success(
            upsert(
                Profile(
                    id = "",
                    name = name,
                    host = host,
                    port = 443,
                    secret = secret,
                    templateId = templates.first().id,
                    latencyMs = Random.nextInt(20, 180),
                ),
            ),
        )
    }

    override suspend fun measureLatency(id: String): Int {
        delay(900)
        val base = state.value.firstOrNull { it.id == id }?.latencyMs ?: 80
        val measured = (base + Random.nextInt(-6, 9)).coerceAtLeast(4)
        state.update { list -> list.map { if (it.id == id) it.copy(latencyMs = measured) else it } }
        return measured
    }

    override fun shareLink(profile: Profile): String =
        "fiend://${profile.secret.take(43)}@${profile.authority}?t=${profile.templateId}#${profile.name.replace(" ", "%20")}"
}
