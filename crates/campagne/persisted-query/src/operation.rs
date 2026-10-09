//! The header of a `.graphql` file: `query <Name>($v: ID!, …)`. Only the
//! signature is read. The selection set documents the shape of the answer and
//! is never interpreted: the SQL sidecar is what runs.

pub(crate) struct Operation {
    pub name: String,
    /// Declared variables, in order. Every one is `ID!`.
    pub variables: Vec<String>,
}

pub(crate) fn parse_header(text: &str) -> Result<Operation, String> {
    let end = text
        .find(['@', '{'])
        .ok_or_else(|| "the query file has no selection set".to_owned())?;
    let header = text[..end].trim();
    let rest = header
        .strip_prefix("query")
        .filter(|r| r.starts_with(char::is_whitespace))
        .ok_or_else(|| "the query file does not open with `query`".to_owned())?
        .trim_start();

    let name_len = rest
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    let (name, rest) = rest.split_at(name_len);
    if !is_name(name) {
        return Err("the operation has no valid name".to_owned());
    }

    let rest = rest.trim();
    let variables = if rest.is_empty() {
        Vec::new()
    } else {
        let inner = rest
            .strip_prefix('(')
            .and_then(|r| r.strip_suffix(')'))
            .ok_or_else(|| "the variable list is malformed".to_owned())?;
        parse_variables(inner)?
    };
    Ok(Operation {
        name: name.to_owned(),
        variables,
    })
}

fn parse_variables(inner: &str) -> Result<Vec<String>, String> {
    let mut variables: Vec<String> = Vec::new();
    for piece in inner.split(',') {
        let (name, ty) = piece
            .trim()
            .strip_prefix('$')
            .and_then(|p| p.split_once(':'))
            .ok_or_else(|| "a variable is malformed".to_owned())?;
        let name = name.trim();
        if !is_name(name) {
            return Err("a variable has no valid name".to_owned());
        }
        if ty.trim() != "ID!" {
            return Err(format!("variable {name} is not of type ID!"));
        }
        if variables.iter().any(|v| v == name) {
            return Err(format!("variable {name} is declared twice"));
        }
        variables.push(name.to_owned());
    }
    Ok(variables)
}

fn is_name(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_variables() {
        let op = parse_header("query ListeCampagnes @capability(id: \"x.y\") {\n a\n}\n").unwrap();
        assert_eq!(op.name, "ListeCampagnes");
        assert!(op.variables.is_empty());
    }

    #[test]
    fn one_id_variable() {
        let op = parse_header("query ListePjs($campagneId: ID!) @capability(id: \"x.y\") { a }")
            .unwrap();
        assert_eq!(op.name, "ListePjs");
        assert_eq!(op.variables, ["campagneId"]);
    }

    #[test]
    fn several_variables_keep_their_order() {
        let op = parse_header("query Q($a: ID!, $b : ID!) { a }").unwrap();
        assert_eq!(op.variables, ["a", "b"]);
    }

    #[test]
    fn refuses_an_id_that_is_not_required() {
        assert!(parse_header("query Q($a: ID) { a }").is_err());
    }

    #[test]
    fn refuses_another_type() {
        assert!(parse_header("query Q($a: Int!) { a }").is_err());
        assert!(parse_header("query Q($a: [ID!]!) { a }").is_err());
    }

    #[test]
    fn refuses_a_missing_query_keyword() {
        assert!(parse_header("mutation Q { a }").is_err());
        assert!(parse_header("Q($a: ID!) { a }").is_err());
        assert!(parse_header("queryQ { a }").is_err());
    }

    #[test]
    fn refuses_a_duplicate_or_malformed_variable() {
        assert!(parse_header("query Q($a: ID!, $a: ID!) { a }").is_err());
        assert!(parse_header("query Q(a: ID!) { a }").is_err());
        assert!(parse_header("query Q($a: ID!").is_err());
    }

    #[test]
    fn refuses_a_file_without_a_selection_set() {
        assert!(parse_header("query Q").is_err());
    }
}
