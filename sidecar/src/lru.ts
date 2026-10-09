/** Conjunto LRU acotado, para deduplicar por `msgId`. */
export class LruSet {
  private readonly items = new Set<string>();

  constructor(private readonly capacity: number) {
    if (capacity < 1) throw new RangeError("capacity must be >= 1");
  }

  get size(): number {
    return this.items.size;
  }

  /** Devuelve `true` si la clave ya estaba (y la marca como reciente); si no, la agrega. */
  seen(key: string): boolean {
    if (this.items.has(key)) {
      this.items.delete(key);
      this.items.add(key);
      return true;
    }
    this.items.add(key);
    if (this.items.size > this.capacity) {
      const oldest = this.items.values().next();
      if (!oldest.done) this.items.delete(oldest.value);
    }
    return false;
  }
}
