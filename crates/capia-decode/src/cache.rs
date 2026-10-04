//! Cache LRU com **orçamento em bytes** (ADR-060). Genérico; o serviço o usa para quadros.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::sync::Arc;

#[derive(Debug)]
struct Slot<V> {
    value: Arc<V>,
    bytes: u64,
    tick: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub rejected_oversize: u64,
    pub entries: usize,
    pub bytes: u64,
}

/// LRU por bytes: `insert` expulsa os menos recentes até caber; um item maior que o orçamento
/// inteiro **não** entra (e nunca expulsa os outros à toa).
#[derive(Debug)]
pub struct ByteLru<K, V> {
    budget: u64,
    used: u64,
    tick: u64,
    map: HashMap<K, Slot<V>>,
    order: BTreeMap<u64, K>,
    stats: CacheStats,
}

impl<K: Hash + Eq + Clone, V> ByteLru<K, V> {
    pub fn new(budget_bytes: u64) -> Self {
        Self {
            budget: budget_bytes,
            used: 0,
            tick: 0,
            map: HashMap::new(),
            order: BTreeMap::new(),
            stats: CacheStats::default(),
        }
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.map.len(),
            bytes: self.used,
            ..self.stats
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// Consulta e marca como o mais recente.
    pub fn get(&mut self, key: &K) -> Option<Arc<V>> {
        let t = self.next_tick();
        match self.map.get_mut(key) {
            Some(slot) => {
                self.order.remove(&slot.tick);
                slot.tick = t;
                self.order.insert(t, key.clone());
                self.stats.hits += 1;
                Some(Arc::clone(&slot.value))
            }
            None => {
                self.stats.misses += 1;
                None
            }
        }
    }

    /// Consulta sem alterar a ordem nem as estatísticas.
    pub fn peek(&self, key: &K) -> Option<Arc<V>> {
        self.map.get(key).map(|s| Arc::clone(&s.value))
    }

    pub fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    pub fn insert(&mut self, key: K, value: Arc<V>, bytes: u64) {
        self.remove(&key);
        if bytes > self.budget {
            self.stats.rejected_oversize += 1;
            return;
        }
        while self.used.saturating_add(bytes) > self.budget {
            let Some((&oldest, _)) = self.order.iter().next() else {
                break;
            };
            if let Some(k) = self.order.remove(&oldest)
                && let Some(slot) = self.map.remove(&k)
            {
                self.used -= slot.bytes;
                self.stats.evictions += 1;
            }
        }
        let t = self.next_tick();
        self.order.insert(t, key.clone());
        self.map.insert(
            key,
            Slot {
                value,
                bytes,
                tick: t,
            },
        );
        self.used += bytes;
    }

    pub fn remove(&mut self, key: &K) -> bool {
        match self.map.remove(key) {
            Some(slot) => {
                self.order.remove(&slot.tick);
                self.used -= slot.bytes;
                true
            }
            None => false,
        }
    }

    /// Remove todas as entradas para as quais `keep` é falso. Devolve quantas saíram.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) -> usize {
        let doomed: Vec<K> = self.map.keys().filter(|k| !keep(k)).cloned().collect();
        for k in &doomed {
            self.remove(k);
        }
        doomed.len()
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.used = 0;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn put(c: &mut ByteLru<u32, Vec<u8>>, k: u32, n: usize) {
        c.insert(k, Arc::new(vec![0; n]), n as u64);
    }

    #[test]
    fn evicts_least_recently_used_until_it_fits() {
        let mut c = ByteLru::new(100);
        put(&mut c, 1, 40);
        put(&mut c, 2, 40);
        assert!(c.get(&1).is_some()); // 1 vira o mais recente
        put(&mut c, 3, 40); // precisa expulsar 2
        assert!(c.contains(&1) && c.contains(&3) && !c.contains(&2));
        assert_eq!(c.used(), 80);
        assert_eq!(c.stats().evictions, 1);
    }

    #[test]
    fn budget_is_never_exceeded_and_oversize_is_rejected() {
        let mut c = ByteLru::new(100);
        for k in 0..50 {
            put(&mut c, k, 30);
            assert!(c.used() <= 100);
        }
        put(&mut c, 999, 101);
        assert!(!c.contains(&999));
        assert_eq!(c.stats().rejected_oversize, 1);
        assert!(c.len() == 3 && c.used() == 90);
    }

    #[test]
    fn reinsert_replaces_and_accounts_once() {
        let mut c = ByteLru::new(100);
        put(&mut c, 1, 60);
        put(&mut c, 1, 20);
        assert_eq!(c.used(), 20);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn hits_and_misses_are_counted_and_retain_filters() {
        let mut c = ByteLru::new(1000);
        put(&mut c, 1, 10);
        put(&mut c, 2, 10);
        assert!(c.get(&1).is_some());
        assert!(c.get(&7).is_none());
        let s = c.stats();
        assert_eq!((s.hits, s.misses), (1, 1));
        assert_eq!(c.retain(|k| *k != 1), 1);
        assert!(!c.contains(&1) && c.contains(&2));
        assert_eq!(c.used(), 10);
    }
}
