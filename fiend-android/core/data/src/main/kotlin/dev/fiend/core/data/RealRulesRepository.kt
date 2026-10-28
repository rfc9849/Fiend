package dev.fiend.core.data

import android.content.Context
import android.net.Uri
import dagger.hilt.android.qualifiers.ApplicationContext
import dev.fiend.core.engine.bindings.inspectRuleFile
import dev.fiend.core.engine.bindings.validateResolver as nativeValidateResolver
import dev.fiend.core.engine.bindings.validateRules
import dev.fiend.core.engine.bindings.RuleKind as NativeRuleKind
import dev.fiend.core.model.RuleCounts
import dev.fiend.core.model.RuleKind
import java.io.ByteArrayOutputStream
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Singleton
internal class RealRulesRepository @Inject constructor(
    @ApplicationContext private val context: Context,
) : RulesRepository {
    override fun validate(text: String, allowIp: Boolean): Result<RuleCounts> = runCatching {
        val stats = validateRules(text, allowIp)
        RuleCounts(stats.domains.toInt(), stats.exact.toInt(), stats.keywords.toInt(), stats.cidrs.toInt())
    }.recoverCatching { throw IllegalArgumentException(it.message ?: "Rules are not valid") }

    override fun validateResolver(value: String): Result<Unit> = runCatching {
        nativeValidateResolver(value.trim())
    }.recoverCatching { throw IllegalArgumentException(it.message ?: "Resolver is not valid") }

    override fun inspectFile(path: String, kind: RuleKind): Result<RuleCounts> = runCatching {
        val stats = inspectRuleFile(
            path,
            when (kind) {
                RuleKind.Direct -> NativeRuleKind.DIRECT
                RuleKind.Block -> NativeRuleKind.BLOCK
                RuleKind.Proxy -> NativeRuleKind.PROXY
            },
        )
        RuleCounts(stats.domains.toInt(), stats.exact.toInt(), stats.keywords.toInt(), stats.cidrs.toInt())
    }.recoverCatching { throw IllegalArgumentException(it.message ?: "Rule file is not valid") }

    override suspend fun read(uri: String): Result<String> = withContext(Dispatchers.IO) {
        runCatching {
            val input = context.contentResolver.openInputStream(Uri.parse(uri)) ?: error("Cannot open the file")
            input.use { stream ->
                val output = ByteArrayOutputStream()
                val buffer = ByteArray(8192)
                while (true) {
                    val size = stream.read(buffer)
                    if (size < 0) break
                    require(output.size() + size <= RulesRepository.MAX_IMPORT_BYTES) { "The file is larger than 1 MiB" }
                    output.write(buffer, 0, size)
                }
                decode(output.toByteArray())
            }
        }
    }

    private fun decode(bytes: ByteArray): String {
        val decoder = Charsets.UTF_8.newDecoder()
        return decoder.decode(java.nio.ByteBuffer.wrap(bytes)).toString().removePrefix("\uFEFF")
    }
}
