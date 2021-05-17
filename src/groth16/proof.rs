use std::io::{self, Read, Write};

use groupy::{CurveAffine, EncodedPoint};

use crate::bls::Engine;


#[derive(Clone, Debug)]
pub struct Proof<E: Engine> {
    pub a: E::G1Affine,
    pub b: E::G2Affine,
    pub c: E::G1Affine,
}

impl<E: Engine> PartialEq for Proof<E> {
    fn eq(&self, other: &Self) -> bool {
        self.a == other.a && self.b == other.b && self.c == other.c
    }
}

impl<E: Engine> Proof<E> {
    pub fn write<W: Write>(&self, mut writer: W) -> io::Result<()> {
        writer.write_all(self.a.into_compressed().as_ref())?;
        writer.write_all(self.b.into_compressed().as_ref())?;
        writer.write_all(self.c.into_compressed().as_ref())?;

        Ok(())
    }

    pub fn read<R: Read>(mut reader: R) -> io::Result<Self> {
        let mut bytes = vec![0u8; Self::size()];
        reader.read_exact(&mut bytes)?;
        let proof = Self::read_many(&bytes, 1)?.pop().unwrap();

        Ok(proof)
    }

    pub fn read_from_byteblob(proof_bytes: &[u8]) -> io::Result<Self> {
        let mut proof_a_repr = <E::G1Affine as CurveAffine>::empty();
        let proof_a_start = 0;
        let proof_a_end = proof_a_start + <E::G1Affine as CurveAffine>::size();
        proof_a_repr.as_mut().copy_from_slice(&proof_bytes[proof_a_start..proof_a_end]);

        let mut proof_b_repr = <E::G1Affine as CurveAffine>::empty();
        let proof_b_start = 0;
        let proof_b_end = proof_b_start + <E::G1Affine as CurveAffine>::size();
        proof_b_repr.as_mut().copy_from_slice(&proof_bytes[proof_b_start..proof_b_end]);

        let mut proof_c_repr = <E::G1Affine as CurveAffine>::empty();
        let proof_c_start = 0;
        let proof_c_end = proof_c_start + <E::G1Affine as CurveAffine>::size();
        proof_c_repr.as_mut().copy_from_slice(&proof_cytes[proof_c_start..proof_c_end]);

        let proof = Proof::<E> {a: proof_a_repr, 
                                b: proof_b_repr,
                                c: proof_c_repr};

        Ok(proof)
    }

    pub fn size() -> usize {
        2 * <<<E as Engine>::G1Affine as groupy::CurveAffine>::Compressed as groupy::EncodedPoint>::size()
            + <<<E as Engine>::G2Affine as groupy::CurveAffine>::Compressed as groupy::EncodedPoint>::size(
            )
    }

    pub fn read_many(proof_bytes: &[u8], num_proofs: usize) -> io::Result<Vec<Self>> {
        use crate::multicore::THREAD_POOL;
        use rayon::prelude::*;
        debug_assert_eq!(proof_bytes.len(), num_proofs * Self::size());

        // Decompress and group check in parallel
        THREAD_POOL.install(|| {
            #[derive(Clone, Copy)]
            enum ProofPart<E: Engine> {
                A(E::G1Affine),
                B(E::G2Affine),
                C(E::G1Affine),
            }

            let parts = (0..num_proofs * 3)
                .into_par_iter()
                .map(|i| -> io::Result<_> {
                    // Work on all G2 points first since they are more expensive. Avoid
                    // having a long pole due to g2 starting late.
                    let c = i / num_proofs;
                    let p = i % num_proofs;
                    let offset = Self::size() * p;
                    match c {
                        0 => {
                            let mut g2_repr = <E::G2Affine as CurveAffine>::Compressed::empty();
                            let start = offset + <E::G1Affine as CurveAffine>::Compressed::size();
                            let end = start + <E::G2Affine as CurveAffine>::Compressed::size();
                            g2_repr.as_mut().copy_from_slice(&proof_bytes[start..end]);

                            let b = g2_repr
                                .into_affine()
                                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
                                .and_then(|e| {
                                    if e.is_zero() {
                                        Err(io::Error::new(
                                            io::ErrorKind::InvalidData,
                                            "point at infinity",
                                        ))
                                    } else {
                                        Ok(e)
                                    }
                                })?;

                            Ok(ProofPart::<E>::B(b))
                        }
                        1 => {
                            let mut g1_repr = <E::G1Affine as CurveAffine>::Compressed::empty();
                            let start = offset;
                            let end = start + <E::G1Affine as CurveAffine>::Compressed::size();
                            g1_repr.as_mut().copy_from_slice(&proof_bytes[start..end]);

                            let a = g1_repr
                                .into_affine()
                                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
                                .and_then(|e| {
                                    if e.is_zero() {
                                        Err(io::Error::new(
                                            io::ErrorKind::InvalidData,
                                            "point at infinity",
                                        ))
                                    } else {
                                        Ok(e)
                                    }
                                })?;
                            Ok(ProofPart::<E>::A(a))
                        }
                        2 => {
                            let mut g1_repr = <E::G1Affine as CurveAffine>::Compressed::empty();
                            let start = offset
                                + <E::G1Affine as CurveAffine>::Compressed::size()
                                + <E::G2Affine as CurveAffine>::Compressed::size();
                            let end = start + <E::G1Affine as CurveAffine>::Compressed::size();

                            g1_repr.as_mut().copy_from_slice(&proof_bytes[start..end]);
                            let c = g1_repr
                                .into_affine()
                                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
                                .and_then(|e| {
                                    if e.is_zero() {
                                        Err(io::Error::new(
                                            io::ErrorKind::InvalidData,
                                            "point at infinity",
                                        ))
                                    } else {
                                        Ok(e)
                                    }
                                })?;

                            Ok(ProofPart::<E>::C(c))
                        }
                        _ => unreachable!("invalid math {}", c),
                    }
                })
                .collect::<io::Result<Vec<_>>>()?;

            let mut proofs = vec![
                Proof::<E> {
                    a: <E::G1Affine as CurveAffine>::zero(),
                    b: <E::G2Affine as CurveAffine>::zero(),
                    c: <E::G1Affine as CurveAffine>::zero(),
                };
                num_proofs
            ];

            for (i, part) in parts.into_iter().enumerate() {
                let c = i / num_proofs;
                let p = i % num_proofs;
                let proof = &mut proofs[p];
                match c {
                    0 => {
                        if let ProofPart::B(b) = part {
                            proof.b = b;
                        } else {
                            unreachable!("invalid construction");
                        };
                    }
                    1 => {
                        if let ProofPart::A(a) = part {
                            proof.a = a;
                        } else {
                            unreachable!("invalid construction");
                        };
                    }
                    2 => {
                        if let ProofPart::C(c) = part {
                            proof.c = c;
                        } else {
                            unreachable!("invalid construction");
                        };
                    }
                    _ => unreachable!("invalid math {}", c),
                }
            }

            Ok(proofs)
        })
    }
}

#[cfg(test)]
mod test_with_bls12_381 {
    use super::*;
    use crate::bls::{Bls12, Fr};
    use crate::groth16::{
        create_random_proof, generate_random_parameters, prepare_verifying_key, verify_proof,
        Parameters,
    };
    use crate::{Circuit, ConstraintSystem, SynthesisError};

    use ff::Field;
    use rand::thread_rng;

    #[test]
    fn test_size() {
        assert_eq!(Proof::<Bls12>::size(), 192);
    }

    #[test]
    fn test_print() {
        println!("Test runs!");

        // let g1_one = paired::bls12_381::G1Affine::one();
        // let a = paired::bls12_381::G1Affine::one();
        // let a = a.mul(g1_one);


        // println!("Print a before proof creation: {}", a);
        // println!("Print b before proof creation: {}", b);
        // println!("Print c before proof creation: {}", c);

        let prf = Proof::<Bls12> {a: paired::bls12_381::G1Affine::one(),
            b: paired::bls12_381::G2Affine::one(),
            c: paired::bls12_381::G1Affine::one()};

        // let rng = &mut thread_rng();

        // let random_proof = Proof {
        //         a: paired::bls12_381::G1Projective::random(&mut rng).into_affine(),
        //         b: paired::bls12_381::G2Projective::random(&mut rng).into_affine(),
        //         c: paired::bls12_381::G1Projective::random(&mut rng).into_affine(),
        //     };

        println!("Print a after proof creation: {}", prf.a);
        println!("Print b after proof creation: {}", prf.b);
        println!("Print c after proof creation: {}", prf.c);
        
        let mut v = vec![];
        prf.write(&mut v).unwrap();

        // let mut v_copy = vec![v];

        println!("Vector size: {}", v.len());
        println!("{:02x?}", v);

        let de_prf = Proof::<Bls12>::read(&v[..]).unwrap();

        println!("Print a after proof decoding: {}", de_prf.a);
        println!("Print b after proof decoding: {}", de_prf.b);
        println!("Print c after proof decoding: {}", de_prf.c);

        assert_eq!(1, 1);
    }

    #[test]
    fn serialization() {
        struct MySillyCircuit<E: Engine> {
            a: Option<E::Fr>,
            b: Option<E::Fr>,
        }

        impl<E: Engine> Circuit<E> for MySillyCircuit<E> {
            fn synthesize<CS: ConstraintSystem<E>>(
                self,
                cs: &mut CS,
            ) -> Result<(), SynthesisError> {
                let a = cs.alloc(|| "a", || self.a.ok_or(SynthesisError::AssignmentMissing))?;
                let b = cs.alloc(|| "b", || self.b.ok_or(SynthesisError::AssignmentMissing))?;
                let c = cs.alloc_input(
                    || "c",
                    || {
                        let mut a = self.a.ok_or(SynthesisError::AssignmentMissing)?;
                        let b = self.b.ok_or(SynthesisError::AssignmentMissing)?;

                        a.mul_assign(&b);
                        Ok(a)
                    },
                )?;

                cs.enforce(|| "a*b=c", |lc| lc + a, |lc| lc + b, |lc| lc + c);

                Ok(())
            }
        }

        let rng = &mut thread_rng();

        let params =
            generate_random_parameters::<Bls12, _, _>(MySillyCircuit { a: None, b: None }, rng)
                .unwrap();

        {
            let mut v = vec![];

            params.write(&mut v).unwrap();
            assert_eq!(v.len(), 2136);

            let de_params = Parameters::read(&v[..], true).unwrap();
            assert!(params == de_params);

            let de_params = Parameters::read(&v[..], false).unwrap();
            assert!(params == de_params);
        }

        let pvk = prepare_verifying_key::<Bls12>(&params.vk);

        for _ in 0..100 {
            let a = Fr::random(rng);
            let b = Fr::random(rng);
            let mut c = a;
            c.mul_assign(&b);

            let proof = create_random_proof(
                MySillyCircuit {
                    a: Some(a),
                    b: Some(b),
                },
                &params,
                rng,
            )
            .unwrap();

            let mut v = vec![];
            proof.write(&mut v).unwrap();

            assert_eq!(v.len(), 192);

            let de_proof = Proof::read(&v[..]).unwrap();
            assert!(proof == de_proof);

            // read two proofs
            proof.write(&mut v).unwrap();
            let de_proofs = Proof::read_many(&v[..], 2).unwrap();
            assert_eq!(de_proofs.len(), 2);
            assert_eq!(de_proofs[0], proof);
            assert_eq!(de_proofs[1], proof);

            assert!(verify_proof(&pvk, &proof, &[c]).unwrap());
            assert!(!verify_proof(&pvk, &proof, &[a]).unwrap());
        }
    }
}
