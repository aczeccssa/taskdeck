export function keepAvailable(current: string, available: readonly string[], remembered = ""): string {
    if (current && available.includes(current)) return current;
    if (remembered && available.includes(remembered)) return remembered;
    return available[0] ?? "";
}
