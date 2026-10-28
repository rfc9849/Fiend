package dev.fiend.core.data

import dagger.Binds
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import android.content.Context
import androidx.room.Room
import dagger.hilt.android.qualifiers.ApplicationContext
import dev.fiend.core.data.storage.ProfileDatabase
import dev.fiend.core.data.storage.SecretCipher
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
    fun profiles(impl: RoomProfileRepository): ProfileRepository

    @Binds
    fun tunnel(impl: RealTunnelController): TunnelController

    @Binds
    fun settings(impl: RealSettingsRepository): SettingsRepository

    @Binds
    fun rules(impl: RealRulesRepository): RulesRepository

    @Binds
    fun subscriptions(impl: RealSubscriptionRepository): SubscriptionRepository

    @Binds
    fun apps(impl: RealAppsRepository): AppsRepository

    @Binds
    fun logs(impl: RealLogRepository): LogRepository

    companion object {
        @Provides
        @Singleton
        fun database(@ApplicationContext context: Context): ProfileDatabase = Room.databaseBuilder(context, ProfileDatabase::class.java, "profiles.db")
            .addMigrations(ProfileDatabase.MIGRATION_1_2).build()

        @Provides
        @Singleton
        fun secretCipher(): SecretCipher = SecretCipher.androidKeystore()

        @Provides
        @Singleton
        @ApplicationScope
        fun applicationScope(): CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    }
}
