//! The syntax outline (0063 §2, 0107 §9 step 5): what each linked
//! grammar's symbols query extracts, with the lines each declaration spans.
use crate::languages;
use crate::symbols::DeclExtractor;
use strop_core::symbol_kind::SymbolKind;

fn extract(language: languages::LanguageId, src: &str) -> Vec<(SymbolKind, String, usize, usize)> {
    let mut extractor = DeclExtractor::for_language(language).expect("extractor");
    extractor
        .extract(src.as_bytes(), &|| false)
        .expect("evidence")
        .into_iter()
        .map(|d| {
            let (first, last) = d.lines();
            (d.kind, d.name, first, last)
        })
        .collect()
}

#[test]
fn rust_declarations() {
    let src = "mod outer {\n    const C: u8 = 1;\n    trait Iface { fn required(); }\n    impl Iface for Foo {\n        fn method(&self) {}\n    }\n}\nfn free_fn() -> u8 {\n    C\n}\nstruct St;\nenum E { A }\n";
    assert_eq!(
        extract(languages::LanguageId::Rust, src),
        vec![
            (SymbolKind::Module, "outer".into(), 1, 7),
            (SymbolKind::Constant, "C".into(), 2, 2),
            (SymbolKind::Interface, "Iface".into(), 3, 3),
            (SymbolKind::Method, "required".into(), 3, 3),
            (SymbolKind::Impl, "impl Iface for Foo".into(), 4, 6),
            (SymbolKind::Method, "method".into(), 5, 5),
            (SymbolKind::Function, "free_fn".into(), 8, 10),
            (SymbolKind::Struct, "St".into(), 11, 11),
            (SymbolKind::Enum, "E".into(), 12, 12),
        ]
    );
}

#[test]
fn python_declarations() {
    let src = "class Cls:\n    def method(self):\n        pass\n\ndef free_fn():\n    return\n";
    assert_eq!(
        extract(languages::LanguageId::Python, src),
        vec![
            (SymbolKind::Class, "Cls".into(), 1, 3),
            (SymbolKind::Method, "method".into(), 2, 3),
            (SymbolKind::Function, "free_fn".into(), 5, 6),
        ]
    );
}

#[test]
fn lua_functions_and_methods() {
    let src =
        "local function free_fn()\nend\nfunction mod.fn2()\nend\nfunction mod:method()\nend\n";
    assert_eq!(
        extract(languages::LanguageId::Lua, src),
        vec![
            (SymbolKind::Function, "free_fn".into(), 1, 2),
            (SymbolKind::Function, "mod.fn2".into(), 3, 4),
            (SymbolKind::Method, "mod:method".into(), 5, 6),
        ]
    );
}

#[test]
fn c_declarations() {
    let src = "struct St { int a; };\nenum E { A };\nint free_fn(int x) { return x; }\nint proto(int);\nstruct Fwd;\nint usage(struct St s);\n";
    let found = extract(languages::LanguageId::C, src);
    assert!(found.contains(&(SymbolKind::Struct, "St".into(), 1, 1)));
    assert!(found.contains(&(SymbolKind::Enum, "E".into(), 2, 2)));
    assert!(found.contains(&(SymbolKind::Function, "free_fn".into(), 3, 3)));
    assert!(found.contains(&(SymbolKind::Function, "proto".into(), 4, 4)));
    // Forward declaration and type usage are not definitions.
    assert!(!found.iter().any(|(_, name, _, _)| name == "Fwd"));
    assert_eq!(found.iter().filter(|(_, n, _, _)| n == "St").count(), 1);
    assert!(found.contains(&(SymbolKind::Function, "usage".into(), 6, 6)));
}

#[test]
fn cpp_classes_and_methods() {
    let src = "class Cls {\npublic:\n    void method() {}\n    int attr;\n};\nvoid free_fn() {}\nint Cls::out_of_line() { return 0; }\n";
    assert_eq!(
        extract(languages::LanguageId::Cpp, src),
        vec![
            (SymbolKind::Class, "Cls".into(), 1, 5),
            (SymbolKind::Method, "method".into(), 3, 3),
            (SymbolKind::Function, "free_fn".into(), 6, 6),
            (SymbolKind::Function, "Cls::out_of_line".into(), 7, 7),
        ]
    );
}

#[test]
fn multiline_spans_cover_bodies() {
    let src = "fn wrapped() {\n    let x = 1;\n    let y = 2;\n}\nlet outside = 3;\n";
    assert_eq!(
        extract(languages::LanguageId::Rust, src),
        vec![(SymbolKind::Function, "wrapped".into(), 1, 4)]
    );
    // The name's byte column is the jump target (rg convention).
    let mut extractor = DeclExtractor::for_language(languages::LanguageId::Rust).unwrap();
    let found = extractor.extract(src.as_bytes(), &|| false).unwrap();
    assert_eq!(found[0].selection, [(0, 3), (0, 10)]);
}

#[test]
fn unsupported_languages_have_no_extractor() {
    assert!(DeclExtractor::for_language(languages::LanguageId::Json).is_none());
    assert!(DeclExtractor::for_language(languages::LanguageId::Markdown).is_none());
}

#[test]
fn cancelled_parse_is_not_evidence() {
    let mut extractor =
        DeclExtractor::for_language(languages::LanguageId::Rust).expect("extractor");
    assert!(extractor.extract(b"fn f() {}", &|| true).is_none());
}

/// Every language with a linked grammar and a symbols query compiles
/// its query: a node name the grammar does not have fails here.
#[test]
fn every_symbols_query_compiles() {
    use languages::LanguageId::*;
    for language in [
        Rust, Python, C, Cpp, Lua, Go, Java, JavaScript, TypeScript, Tsx, CSharp, Php, Ruby,
    ] {
        assert!(DeclExtractor::supports(language), "{language:?}");
        assert!(
            DeclExtractor::for_language(language).is_some(),
            "{language:?}'s query compiles"
        );
    }
}

#[test]
fn rust_impl_blocks_are_named_as_rust_analyzer_names_them() {
    let src = "impl Modeline {
    fn fit(&self) {}
}
impl<T: Clone> std::fmt::Display
    for Wrapper<T>
{
}
";
    assert_eq!(
        extract(languages::LanguageId::Rust, src),
        vec![
            (SymbolKind::Impl, "impl Modeline".into(), 1, 3),
            (SymbolKind::Method, "fit".into(), 2, 2),
            (
                SymbolKind::Impl,
                "impl std::fmt::Display for Wrapper<T>".into(),
                4,
                7
            ),
        ]
    );
    assert_eq!(SymbolKind::Impl.chip(), "impl");
    assert!(SymbolKind::Impl.spec().lsp.contains(&"Object"));
}

#[test]
fn go_declarations() {
    let src = "package p

type Shape interface {
	Area() int
}

type Box struct {
	w int
}

func (b *Box) Area() int {
	return b.w
}

func main() {
}
";
    assert_eq!(
        extract(languages::LanguageId::Go, src),
        vec![
            (SymbolKind::Interface, "Shape".into(), 3, 5),
            (SymbolKind::Method, "Area".into(), 4, 4),
            (SymbolKind::Struct, "Box".into(), 7, 9),
            (SymbolKind::Method, "Area".into(), 11, 13),
            (SymbolKind::Function, "main".into(), 15, 16),
        ]
    );
}

#[test]
fn java_declarations() {
    let src = "class Shape {
    Shape() {}
    int area() {
        return 0;
    }
}
interface Named {}
enum Color { RED }
";
    assert_eq!(
        extract(languages::LanguageId::Java, src),
        vec![
            (SymbolKind::Class, "Shape".into(), 1, 6),
            (SymbolKind::Method, "Shape".into(), 2, 2),
            (SymbolKind::Method, "area".into(), 3, 5),
            (SymbolKind::Interface, "Named".into(), 7, 7),
            (SymbolKind::Enum, "Color".into(), 8, 8),
        ]
    );
}

#[test]
fn javascript_declarations() {
    let src = "class Shape {
  area() {
    return 0;
  }
}
function free() {}
const arrow = () => 1;
";
    assert_eq!(
        extract(languages::LanguageId::JavaScript, src),
        vec![
            (SymbolKind::Class, "Shape".into(), 1, 5),
            (SymbolKind::Method, "area".into(), 2, 4),
            (SymbolKind::Function, "free".into(), 6, 6),
            (SymbolKind::Function, "arrow".into(), 7, 7),
        ]
    );
}

#[test]
fn typescript_and_tsx_declarations() {
    let src = "namespace Geo {
  export interface Shape {
    area(): number;
  }
  export class Box implements Shape {
    area(): number {
      return 1;
    }
  }
}
enum Color { Red }
function free(): void {}
";
    let expected = vec![
        (SymbolKind::Module, "Geo".into(), 1, 10),
        (SymbolKind::Interface, "Shape".into(), 2, 4),
        (SymbolKind::Method, "area".into(), 3, 3),
        (SymbolKind::Class, "Box".into(), 5, 9),
        (SymbolKind::Method, "area".into(), 6, 8),
        (SymbolKind::Enum, "Color".into(), 11, 11),
        (SymbolKind::Function, "free".into(), 12, 12),
    ];
    assert_eq!(extract(languages::LanguageId::TypeScript, src), expected);
    assert_eq!(extract(languages::LanguageId::Tsx, src), expected);
}

#[test]
fn csharp_declarations() {
    let src = "namespace Geo {
    class Box {
        Box() {}
        int Area() {
            return 1;
        }
    }
    interface IShape {}
    struct Point {}
    enum Color { Red }
}
";
    assert_eq!(
        extract(languages::LanguageId::CSharp, src),
        vec![
            (SymbolKind::Module, "Geo".into(), 1, 11),
            (SymbolKind::Class, "Box".into(), 2, 7),
            (SymbolKind::Method, "Box".into(), 3, 3),
            (SymbolKind::Method, "Area".into(), 4, 6),
            (SymbolKind::Interface, "IShape".into(), 8, 8),
            (SymbolKind::Struct, "Point".into(), 9, 9),
            (SymbolKind::Enum, "Color".into(), 10, 10),
        ]
    );
}

#[test]
fn php_declarations() {
    let src = "<?php
namespace App;
class Box {
    public function area() {
        return 1;
    }
}
interface Shape {}
trait Named {}
function free() {}
";
    assert_eq!(
        extract(languages::LanguageId::Php, src),
        vec![
            (SymbolKind::Module, "App".into(), 2, 2),
            (SymbolKind::Class, "Box".into(), 3, 7),
            (SymbolKind::Method, "area".into(), 4, 6),
            (SymbolKind::Interface, "Shape".into(), 8, 8),
            (SymbolKind::Interface, "Named".into(), 9, 9),
            (SymbolKind::Function, "free".into(), 10, 10),
        ]
    );
}

#[test]
fn ruby_declarations() {
    let src = "module Geo
  class Box
    def area
      1
    end

    def self.make
    end
  end
end

def free
end
";
    assert_eq!(
        extract(languages::LanguageId::Ruby, src),
        vec![
            (SymbolKind::Module, "Geo".into(), 1, 10),
            (SymbolKind::Class, "Box".into(), 2, 9),
            (SymbolKind::Method, "area".into(), 3, 5),
            (SymbolKind::Method, "make".into(), 7, 8),
            (SymbolKind::Function, "free".into(), 12, 13),
        ]
    );
}
