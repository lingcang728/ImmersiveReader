export type NavigationSnapshot = {
	generation: number;
	path: string;
};

export function isCurrentNavigation(
	expected: NavigationSnapshot,
	actual: NavigationSnapshot,
): boolean {
	return expected.generation === actual.generation && expected.path === actual.path;
}
