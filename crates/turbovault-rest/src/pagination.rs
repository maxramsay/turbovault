use serde::Deserialize;

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;

fn default_limit() -> usize {
    DEFAULT_LIMIT
}

#[derive(Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

impl PaginationParams {
    pub fn validated(&self) -> Self {
        Self {
            limit: self.limit.min(MAX_LIMIT),
            offset: self.offset,
        }
    }
}

pub fn paginate<T: Clone>(items: Vec<T>, params: &PaginationParams) -> (Vec<T>, usize, bool) {
    let total = items.len();
    let params = params.validated();
    let page = items
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .collect();
    let has_more = params.offset + params.limit < total;
    (page, total, has_more)
}
