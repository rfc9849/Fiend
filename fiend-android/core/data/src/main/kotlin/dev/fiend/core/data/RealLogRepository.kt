package dev.fiend.core.data

import dev.fiend.core.model.LogLine
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

@Singleton
class RealLogRepository @Inject constructor() : LogRepository {
    private val mutableLines = MutableStateFlow<List<LogLine>>(emptyList())
    override val lines = mutableLines.asStateFlow()
    override fun append(line: LogLine) { mutableLines.update { (it + line).takeLast(1_000) } }
    override fun clear() { mutableLines.value = emptyList() }
}
