package dev.fiend.core.data

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import dev.fiend.core.model.RuleFileSpec
import dev.fiend.core.model.RuleKind
import dev.fiend.core.model.RuleSubscription
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

@Singleton
internal class RealSubscriptionRepository @Inject constructor(
    @ApplicationContext private val context: Context,
    private val settings: SettingsRepository,
    private val rules: RulesRepository,
) : SubscriptionRepository {
    private val mutex = Mutex()

    private val directory: File
        get() = File(context.filesDir, "rules").apply { mkdirs() }

    private fun fileFor(id: String) = File(directory, "$id.bin")

    override suspend fun ruleFiles(): List<RuleFileSpec> = withContext(Dispatchers.IO) {
        settings.awaitReady()
        settings.settings.value.subscriptions
            .filter { fileFor(it.id).isFile }
            .map { RuleFileSpec(fileFor(it.id).absolutePath, it.kind) }
    }

    override suspend fun refreshStale() {
        settings.awaitReady()
        mutex.withLock {
            val now = System.currentTimeMillis()
            val stale = settings.settings.value.subscriptions.filter {
                it.updatedAtMs == 0L || now - it.updatedAtMs >= SubscriptionRepository.REFRESH_INTERVAL_MS
            }
            for (subscription in stale) {
                refreshLocked(subscription)
            }
        }
    }

    override suspend fun refreshAll(): Result<Unit> {
        settings.awaitReady()
        return mutex.withLock {
            var failure: Throwable? = null
            for (subscription in settings.settings.value.subscriptions) {
                refreshLocked(subscription).onFailure { failure = it }
            }
            failure?.let { Result.failure(it) } ?: Result.success(Unit)
        }
    }

    override suspend fun refresh(id: String): Result<Unit> {
        settings.awaitReady()
        return mutex.withLock {
            val subscription = settings.settings.value.subscriptions.firstOrNull { it.id == id }
                ?: return@withLock Result.failure(IllegalArgumentException("The subscription is not available"))
            refreshLocked(subscription)
        }
    }

    override suspend fun add(url: String, kind: RuleKind, name: String): Result<RuleSubscription> {
        settings.awaitReady()
        return mutex.withLock {
            val link = runCatching { validateUrl(url) }.getOrElse { return@withLock Result.failure(it) }
            val clean = runCatching { validateName(name) }.getOrElse { return@withLock Result.failure(it) }
            val subscription = RuleSubscription(id = UUID.randomUUID().toString(), url = link, kind = kind, name = clean)
            settings.update { it.copy(subscriptions = it.subscriptions + subscription) }
            refreshLocked(subscription)
            Result.success(settings.settings.value.subscriptions.first { it.id == subscription.id })
        }
    }

    override suspend fun edit(id: String, name: String, kind: RuleKind): Result<RuleSubscription> {
        settings.awaitReady()
        return mutex.withLock {
            val clean = runCatching { validateName(name) }.getOrElse { return@withLock Result.failure(it) }
            val current = settings.settings.value.subscriptions.firstOrNull { it.id == id }
                ?: return@withLock Result.failure(IllegalArgumentException("The subscription is not available"))
            val file = fileFor(id)
            val checked = if (current.kind != kind) {
                withContext(Dispatchers.IO) { if (file.isFile) rules.inspectFile(file.absolutePath, kind) else null }
            } else {
                null
            }
            update(id) { item ->
                val changed = item.copy(name = clean, kind = kind)
                when {
                    checked == null -> changed
                    checked.isSuccess -> changed.copy(entries = checked.getOrThrow().total, error = null)
                    else -> changed.copy(error = checked.exceptionOrNull()?.message ?: "The list is not valid")
                }
            }
            Result.success(settings.settings.value.subscriptions.first { it.id == id })
        }
    }

    override suspend fun remove(id: String) {
        settings.awaitReady()
        mutex.withLock {
            settings.update { current -> current.copy(subscriptions = current.subscriptions.filterNot { it.id == id }) }
            withContext(Dispatchers.IO) { fileFor(id).delete() }
        }
    }

    private suspend fun refreshLocked(subscription: RuleSubscription): Result<Unit> {
        val result = withContext(Dispatchers.IO) {
            runCatching {
                val bytes = download(subscription.url)
                val temporary = File(directory, "${subscription.id}.tmp")
                temporary.writeBytes(bytes)
                try {
                    val counts = rules.inspectFile(temporary.absolutePath, subscription.kind).getOrThrow()
                    if (!temporary.renameTo(fileFor(subscription.id))) {
                        temporary.copyTo(fileFor(subscription.id), overwrite = true)
                        temporary.delete()
                    }
                    counts
                } catch (error: Throwable) {
                    temporary.delete()
                    throw error
                }
            }
        }
        result
            .onSuccess { counts ->
                update(subscription.id) {
                    it.copy(updatedAtMs = System.currentTimeMillis(), entries = counts.total, error = null)
                }
            }
            .onFailure { error ->
                update(subscription.id) { it.copy(error = error.message ?: "Cannot download the list") }
            }
        return result.map { }
    }

    private suspend fun update(id: String, transform: (RuleSubscription) -> RuleSubscription) {
        settings.update { current ->
            current.copy(subscriptions = current.subscriptions.map { if (it.id == id) transform(it) else it })
        }
    }

    private fun download(url: String): ByteArray {
        val connection = URI(url).toURL().openConnection() as HttpURLConnection
        try {
            connection.instanceFollowRedirects = true
            connection.connectTimeout = 15_000
            connection.readTimeout = 30_000
            connection.setRequestProperty("User-Agent", "fiend")
            connection.setRequestProperty("Accept", "*/*")
            val code = connection.responseCode
            if (code != HttpURLConnection.HTTP_OK) throw IOException("HTTP $code")
            val output = ByteArrayOutputStream()
            connection.inputStream.use { input ->
                val buffer = ByteArray(16 * 1024)
                while (true) {
                    val size = input.read(buffer)
                    if (size < 0) break
                    if (output.size() + size > SubscriptionRepository.MAX_DOWNLOAD_BYTES) {
                        throw IOException("The list is larger than 32 MiB")
                    }
                    output.write(buffer, 0, size)
                }
            }
            if (output.size() == 0) throw IOException("The list is empty")
            return output.toByteArray()
        } finally {
            connection.disconnect()
        }
    }

    private fun validateName(name: String): String {
        val trimmed = name.trim()
        require(trimmed.length <= RuleSubscription.MAX_NAME && trimmed.none { it.isISOControl() }) {
            "Name must be ${RuleSubscription.MAX_NAME} characters or less, on one line"
        }
        return trimmed
    }

    private fun validateUrl(url: String): String {
        val trimmed = url.trim()
        require(trimmed.length in 1..2048) { "The link is not valid" }
        val parsed = URI(trimmed)
        require(parsed.scheme == "https" || parsed.scheme == "http") { "The link must use HTTP or HTTPS" }
        require(!parsed.host.isNullOrEmpty()) { "The link is not valid" }
        return trimmed
    }
}
