import type { SessionInfo } from '#lib/api/generated/index.ts';

export const session = $state<{ info: SessionInfo | null }>({ info: null });
