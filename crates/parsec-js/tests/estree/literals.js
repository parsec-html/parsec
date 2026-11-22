'single'; "double"; 'it\'s'; "a\"b";
"\b\f\n\r\t\v\0\\";
"\x41B\u{43}\u{1F600}";
"😀"; "\uD800"; "\uDC00x";
"line\
continuation";
"\
";
"  ";
0; 1; 1.5; .5; 5.; 1e3; 1E-3; 1.5e+3; 0x1F; 0X1f; 0o17; 0O17; 0b101; 0B101;
1_000_000; 0x1_0; 1e1_0;
9007199254740993; 1e21; 1e400; 0.000001; 1e-7;
0n; 123n; 0x1Fn; 0o17n; 0b101n; 1_000n; 18446744073709551616n;
/abc/; /a\/b/gimsuy; /[/]/; /(?<name>x)\k<name>/u; /x/v; /a/d;
``; `plain`; `a${b}c${d}e`; `\nA\x41\``; `line
break`; `${a}${b}`;
tag`\unicode and \u{55}`; tag`\01`; tag`\xg${a}\u{110000}`;
`\
`;
