use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocumentPayload {
    pub format: String,
    pub metadata: Map<String, Value>,
    pub pages: Vec<PagePayload>,
    #[serde(default)]
    pub stories: Vec<StoryPayload>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoryPayload {
    pub kind: String,
    pub blocks: Vec<BlockPayload>,
    pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PagePayload {
    pub number: usize,
    pub blocks: Vec<BlockPayload>,
    pub links: Vec<LinkPayload>,
    pub images: Vec<ImagePayload>,
    pub tables: Vec<TablePayload>,
    pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockPayload {
    pub kind: String,
    pub text: String,
    pub bbox: Option<[f64; 4]>,
    pub level: Option<u8>,
    pub style: Option<String>,
    pub links: Vec<LinkPayload>,
    pub images: Vec<ImagePayload>,
    pub table: Option<TablePayload>,
    pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinkPayload {
    pub uri: String,
    pub text: String,
    pub bbox: Option<[f64; 4]>,
    pub page: Option<usize>,
    pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImagePayload {
    pub name: String,
    pub data: String,
    pub media_type: String,
    pub alt: String,
    pub bbox: Option<[f64; 4]>,
    pub page: Option<usize>,
    pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TablePayload {
    pub rows: Vec<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<Vec<CellPayload>>,
    pub bbox: Option<[f64; 4]>,
    pub page: Option<usize>,
    pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CellPayload {
    pub blocks: Vec<BlockPayload>,
    pub col_span: usize,
    pub row_span: usize,
    pub covered: bool,
    pub metadata: Map<String, Value>,
}

impl BlockPayload {
    pub fn paragraph(text: String) -> Self {
        Self {
            kind: "paragraph".into(),
            text,
            ..Self::default()
        }
    }
}
