use crate::domain::ports::intel_repository::{IntelRepository, IntelStats, TagCount};
use crate::domain::values::category::Category;
use std::sync::Arc;

pub struct StatsUseCase {
    repo: Arc<dyn IntelRepository>,
}

impl StatsUseCase {
    pub fn new(repo: Arc<dyn IntelRepository>) -> Self {
        Self { repo }
    }

    pub fn stats(&self) -> Result<IntelStats, String> {
        self.repo.stats()
    }

    pub fn tags(&self, category: Option<Category>) -> Result<Vec<TagCount>, String> {
        self.repo.tags(category)
    }
}
