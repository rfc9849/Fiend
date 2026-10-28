package dev.fiend.core.data

import dagger.Binds
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import dev.fiend.core.data.mock.MockAppsRepository
import dev.fiend.core.data.mock.MockLogRepository
import dev.fiend.core.data.mock.MockProfileRepository
import dev.fiend.core.data.mock.MockSettingsRepository
import dev.fiend.core.data.mock.MockTunnelController
import javax.inject.Qualifier
import javax.inject.Singleton
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

@Qualifier
@Retention(AnnotationRetention.BINARY)
annotation class ApplicationScope

@Module
@InstallIn(SingletonComponent::class)
internal interface DataModule {
    @Binds
    fun profiles(impl: MockProfileRepository): ProfileRepository

    @Binds
    fun tunnel(impl: MockTunnelController): TunnelController

    @Binds
    fun settings(impl: MockSettingsRepository): SettingsRepository

    @Binds
    fun apps(impl: MockAppsRepository): AppsRepository

    @Binds
    fun logs(impl: MockLogRepository): LogRepository

    companion object {
        @Provides
        @Singleton
        @ApplicationScope
        fun applicationScope(): CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    }
}
