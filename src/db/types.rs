//! Built-in PostgreSQL type information (by OID). Types not listed here are
//! resolved from `pg_type` when a database is first browsed.

/// Coarse type category, mirrors `pg_type.typcategory`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cat {
    Bool,
    Number,
    String,
    DateTime,
    Json,
    Binary,
    Uuid,
    Network,
    Geometric,
    Array,
    Other,
}

impl Cat {
    pub fn from_typcategory(c: &str) -> Cat {
        match c {
            "B" => Cat::Bool,
            "N" => Cat::Number,
            "S" => Cat::String,
            "D" | "T" => Cat::DateTime,
            "A" => Cat::Array,
            "I" => Cat::Network,
            "G" => Cat::Geometric,
            _ => Cat::Other,
        }
    }
    pub fn right_aligned(self) -> bool {
        self == Cat::Number
    }
}

pub fn builtin(oid: u32) -> Option<(&'static str, Cat)> {
    use Cat::*;
    Some(match oid {
        16 => ("boolean", Bool),
        17 => ("bytea", Binary),
        18 => ("char", String),
        19 => ("name", String),
        20 => ("bigint", Number),
        21 => ("smallint", Number),
        23 => ("integer", Number),
        24 => ("regproc", Other),
        25 => ("text", String),
        26 => ("oid", Number),
        28 => ("xid", Number),
        114 => ("json", Json),
        142 => ("xml", String),
        194 => ("pg_node_tree", String),
        600 => ("point", Geometric),
        650 => ("cidr", Network),
        700 => ("real", Number),
        701 => ("double precision", Number),
        790 => ("money", Number),
        829 => ("macaddr", Network),
        869 => ("inet", Network),
        1000 => ("boolean[]", Array),
        1001 => ("bytea[]", Array),
        1005 => ("smallint[]", Array),
        1007 => ("integer[]", Array),
        1009 => ("text[]", Array),
        1014 => ("character[]", Array),
        1015 => ("character varying[]", Array),
        1016 => ("bigint[]", Array),
        1021 => ("real[]", Array),
        1022 => ("double precision[]", Array),
        1028 => ("oid[]", Array),
        1034 => ("aclitem[]", Array),
        1042 => ("character", String),
        1043 => ("character varying", String),
        1082 => ("date", DateTime),
        1083 => ("time", DateTime),
        1114 => ("timestamp", DateTime),
        1115 => ("timestamp[]", Array),
        1182 => ("date[]", Array),
        1184 => ("timestamptz", DateTime),
        1185 => ("timestamptz[]", Array),
        1186 => ("interval", DateTime),
        1231 => ("numeric[]", Array),
        1266 => ("timetz", DateTime),
        1560 => ("bit", Other),
        1562 => ("varbit", Other),
        1700 => ("numeric", Number),
        2205 => ("regclass", Other),
        2206 => ("regtype", Other),
        2249 => ("record", Other),
        2278 => ("void", Other),
        2950 => ("uuid", Uuid),
        2951 => ("uuid[]", Array),
        3220 => ("pg_lsn", Other),
        3614 => ("tsvector", String),
        3615 => ("tsquery", String),
        3802 => ("jsonb", Json),
        3807 => ("jsonb[]", Array),
        3904 => ("int4range", Other),
        3906 => ("numrange", Other),
        3908 => ("tsrange", Other),
        3910 => ("tstzrange", Other),
        3912 => ("daterange", Other),
        3926 => ("int8range", Other),
        4072 => ("jsonpath", String),
        5038 => ("pg_snapshot", Other),
        _ => return None,
    })
}
