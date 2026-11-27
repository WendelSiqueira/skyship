//! Guarda um segredo na memória do app sem deixá-lo inteiro e legível.
//!
//! O segredo é separado em duas partes: uma sequência aleatória (gerada pelo sistema operacional, nova a
//! cada segredo) e o resultado de misturá-la com o segredo (XOR). Cada parte vive numa área de memória
//! própria, e nenhuma delas, sozinha, contém o texto. Só `open` junta as duas, no momento do uso, e devolve
//! uma cópia que se apaga da memória ao ser descartada. Ao descartar um `Sealed`, as duas partes também
//! são zeradas.
//!
//! Isso é OFUSCAÇÃO, não criptografia forte: as duas partes estão no mesmo processo, então quem consegue
//! ler toda a memória do app enquanto ele roda consegue juntá-las. O ganho é contra vazamentos
//! acidentais, onde alguém procura o texto da chave num despejo de memória: arquivo de crash (core dump),
//! memória trocada para o disco (swap) ou ferramentas que varrem padrões como `AKIA...`.

use zeroize::{Zeroize, Zeroizing};

pub struct Sealed {
    pad: Zeroizing<Vec<u8>>,
    masked: Zeroizing<Vec<u8>>,
}

impl Sealed {
    /// Guarda `plain` e apaga o original da memória.
    pub fn new(mut plain: String) -> Self {
        let mut pad = vec![0u8; plain.len()];
        getrandom::fill(&mut pad).expect("o sistema não forneceu números aleatórios");
        let masked: Vec<u8> = plain.bytes().zip(&pad).map(|(b, p)| b ^ p).collect();
        plain.zeroize();
        Self {
            pad: Zeroizing::new(pad),
            masked: Zeroizing::new(masked),
        }
    }

    /// Junta as duas partes. O resultado se apaga da memória quando é descartado: use e solte logo.
    pub fn open(&self) -> Zeroizing<String> {
        let plain: Vec<u8> = self.masked.iter().zip(self.pad.iter()).map(|(m, p)| m ^ p).collect();
        match String::from_utf8(plain) {
            Ok(s) => Zeroizing::new(s),
            Err(e) => {
                let mut bytes = e.into_bytes();
                bytes.zeroize();
                Zeroizing::new(String::new())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Sealed;

    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    #[test]
    fn opens_to_the_same_text() {
        let s = Sealed::new(SECRET.to_string());
        assert_eq!(&*s.open(), SECRET);
        assert_eq!(&*s.open(), SECRET, "abrir não consome nem altera");
        assert_eq!(&*Sealed::new(String::new()).open(), "");
        assert_eq!(&*Sealed::new("açaí 🔑".to_string()).open(), "açaí 🔑");
    }

    #[test]
    fn neither_stored_part_contains_the_text() {
        let s = Sealed::new(SECRET.to_string());
        assert_eq!(s.masked.len(), SECRET.len());
        for part in [&s.masked, &s.pad] {
            let hay = String::from_utf8_lossy(part);
            assert!(!hay.contains(SECRET));
            // nem pedaços do texto (um varredor de padrões acharia 6 bytes seguidos de uma chave real)
            assert!(SECRET.as_bytes().windows(6).all(|w| !part.windows(6).any(|p| p == w)));
        }
    }

    #[test]
    fn every_secret_gets_its_own_random_pad() {
        let a = Sealed::new(SECRET.to_string());
        let b = Sealed::new(SECRET.to_string());
        assert_ne!(*a.pad, *b.pad, "a sequência aleatória é nova a cada segredo");
        assert_ne!(*a.masked, *b.masked);
        assert_eq!(&*a.open(), &*b.open());
    }
}
