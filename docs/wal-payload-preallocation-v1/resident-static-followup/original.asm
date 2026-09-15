
/mnt/data/kv9-work/batch-index-coalescing-development-20260915-first/fused/jemalloc-test-executable:     file format elf64-x86-64


Disassembly of section .text:

0000000000419ab0 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins>:
  419ab0:	push   %rbp
  419ab1:	push   %r15
  419ab3:	push   %r14
  419ab5:	push   %r13
  419ab7:	push   %r12
  419ab9:	push   %rbx
  419aba:	sub    $0x78,%rsp
  419abe:	mov    %ecx,%ebx
  419ac0:	mov    %rdx,%r15
  419ac3:	mov    %rsi,%r12
  419ac6:	mov    %rdi,%r14
  419ac9:	cmpl   $0x1,(%rdi)
  419acc:	jne    419b6b <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0xbb>
  419ad2:	add    $0x8,%r14
  419ad6:	mov    (%r14),%rax
  419ad9:	add    $0xfffffffffffffff8,%rax
  419add:	mov    %r14,0x8(%rsp)
  419ae2:	lea    0x10(%rsp),%r13
  419ae7:	mov    %rax,0x10(%rsp)
  419aec:	lea    0xfc0d(%rip),%rax        # 429700 <triomphe::arc::Arc<T>::as_ptr>
  419af3:	mov    %rax,0x18(%rsp)
  419af8:	mov    %r13,%rdi
  419afb:	call   393f50 <<archery::shared_pointer::kind::arct::ArcTK as archery::shared_pointer::kind::SharedPointerKind>::make_mut::{{closure}}>
  419b00:	mov    %rax,%r14
  419b03:	mov    %r13,%rdi
  419b06:	call   *0x18(%rsp)
  419b0a:	mov    0x8(%rsp),%rcx
  419b0f:	mov    %rax,(%rcx)
  419b12:	mov    0x20(%r14),%rax
  419b16:	mov    0x8(%r12),%rdi
  419b1b:	mov    0x10(%r12),%rcx
  419b20:	mov    0x8(%rax),%rsi
  419b24:	mov    0x10(%rax),%rdx
  419b28:	mov    %rcx,%r13
  419b2b:	sub    %rdx,%r13
  419b2e:	cmovb  %rcx,%rdx
  419b32:	call   *0x538b28(%rip)        # 952660 <memcmp@GLIBC_2.2.5>
  419b38:	cltq
  419b3a:	test   %eax,%eax
  419b3c:	cmovne %rax,%r13
  419b40:	test   %r13,%r13
  419b43:	sets   %cl
  419b46:	setg   %al
  419b49:	sub    %cl,%al
  419b4b:	je     419d62 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x2b2>
  419b51:	movzbl %al,%eax
  419b54:	cmp    $0x1,%eax
  419b57:	jne    419e1b <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x36b>
  419b5d:	mov    %r14,%rdi
  419b60:	add    $0x10,%rdi
  419b64:	xor    %ebp,%ebp
  419b66:	jmp    419e20 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x370>
  419b6b:	test   %bl,%bl
  419b6d:	je     419c58 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x1a8>
  419b73:	mov    0x10(%r12),%rax
  419b78:	mov    %rax,0x50(%rsp)
  419b7d:	movups (%r12),%xmm0
  419b82:	movaps %xmm0,0x40(%rsp)
  419b87:	mov    0x10(%r15),%rax
  419b8b:	mov    %rax,0x68(%rsp)
  419b90:	movups (%r15),%xmm1
  419b94:	movups %xmm1,0x58(%rsp)
  419b99:	movq   $0x1,0x8(%rsp)
  419ba2:	movups %xmm0,0x10(%rsp)
  419ba7:	mov    0x50(%rsp),%rax
  419bac:	mov    %rax,0x20(%rsp)
  419bb1:	mov    0x58(%rsp),%rax
  419bb6:	mov    %rax,0x28(%rsp)
  419bbb:	mov    0x60(%rsp),%rax
  419bc0:	mov    %rax,0x30(%rsp)
  419bc5:	mov    0x68(%rsp),%rax
  419bca:	mov    %rax,0x38(%rsp)
  419bcf:	mov    $0x38,%edi
  419bd4:	call   *0x537116(%rip)        # 950cf0 <_DYNAMIC+0x228>
  419bda:	test   %rax,%rax
  419bdd:	je     419e56 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x3a6>
  419be3:	mov    0x38(%rsp),%rcx
  419be8:	mov    %rcx,0x30(%rax)
  419bec:	movups 0x8(%rsp),%xmm0
  419bf1:	movups 0x18(%rsp),%xmm1
  419bf6:	movups 0x28(%rsp),%xmm2
  419bfb:	movups %xmm2,0x20(%rax)
  419bff:	movups %xmm1,0x10(%rax)
  419c03:	movups %xmm0,(%rax)
  419c06:	add    $0x8,%rax
  419c0a:	movq   $0x1,0x8(%rsp)
  419c13:	movq   $0x0,0x10(%rsp)
  419c1c:	movq   $0x0,0x20(%rsp)
  419c25:	mov    %rax,0x30(%rsp)
  419c2a:	movb   $0x1,0x38(%rsp)
  419c2f:	mov    $0x38,%edi
  419c34:	call   *0x5370b6(%rip)        # 950cf0 <_DYNAMIC+0x228>
  419c3a:	test   %rax,%rax
  419c3d:	jne    419d28 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x278>
  419c43:	mov    $0x8,%edi
  419c48:	mov    $0x38,%esi
  419c4d:	call   *0x5370e5(%rip)        # 950d38 <_DYNAMIC+0x270>
  419c53:	jmp    419e9c <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x3ec>
  419c58:	mov    0x10(%r12),%rax
  419c5d:	mov    %rax,0x50(%rsp)
  419c62:	movups (%r12),%xmm0
  419c67:	movaps %xmm0,0x40(%rsp)
  419c6c:	mov    0x10(%r15),%rax
  419c70:	mov    %rax,0x68(%rsp)
  419c75:	movups (%r15),%xmm1
  419c79:	movups %xmm1,0x58(%rsp)
  419c7e:	movq   $0x1,0x8(%rsp)
  419c87:	movups %xmm0,0x10(%rsp)
  419c8c:	mov    0x50(%rsp),%rax
  419c91:	mov    %rax,0x20(%rsp)
  419c96:	mov    0x58(%rsp),%rax
  419c9b:	mov    %rax,0x28(%rsp)
  419ca0:	mov    0x60(%rsp),%rax
  419ca5:	mov    %rax,0x30(%rsp)
  419caa:	mov    0x68(%rsp),%rax
  419caf:	mov    %rax,0x38(%rsp)
  419cb4:	mov    $0x38,%edi
  419cb9:	call   *0x537031(%rip)        # 950cf0 <_DYNAMIC+0x228>
  419cbf:	test   %rax,%rax
  419cc2:	je     419e68 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x3b8>
  419cc8:	mov    0x38(%rsp),%rcx
  419ccd:	mov    %rcx,0x30(%rax)
  419cd1:	movups 0x8(%rsp),%xmm0
  419cd6:	movups 0x18(%rsp),%xmm1
  419cdb:	movups 0x28(%rsp),%xmm2
  419ce0:	movups %xmm2,0x20(%rax)
  419ce4:	movups %xmm1,0x10(%rax)
  419ce8:	movups %xmm0,(%rax)
  419ceb:	add    $0x8,%rax
  419cef:	movq   $0x1,0x8(%rsp)
  419cf8:	movq   $0x0,0x10(%rsp)
  419d01:	movq   $0x0,0x20(%rsp)
  419d0a:	mov    %rax,0x30(%rsp)
  419d0f:	movb   $0x0,0x38(%rsp)
  419d14:	mov    $0x38,%edi
  419d19:	call   *0x536fd1(%rip)        # 950cf0 <_DYNAMIC+0x228>
  419d1f:	test   %rax,%rax
  419d22:	je     419e7a <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x3ca>
  419d28:	mov    0x38(%rsp),%rcx
  419d2d:	mov    %rcx,0x30(%rax)
  419d31:	movups 0x8(%rsp),%xmm0
  419d36:	movups 0x18(%rsp),%xmm1
  419d3b:	movups 0x28(%rsp),%xmm2
  419d40:	movups %xmm2,0x20(%rax)
  419d44:	movups %xmm1,0x10(%rax)
  419d48:	movups %xmm0,(%rax)
  419d4b:	add    $0x8,%rax
  419d4f:	movq   $0x1,(%r14)
  419d56:	mov    %rax,0x8(%r14)
  419d5a:	mov    $0x1,%bpl
  419d5d:	jmp    419e45 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x395>
  419d62:	mov    0x10(%r12),%rax
  419d67:	mov    %rax,0x50(%rsp)
  419d6c:	movups (%r12),%xmm0
  419d71:	movaps %xmm0,0x40(%rsp)
  419d76:	mov    0x10(%r15),%rax
  419d7a:	mov    %rax,0x68(%rsp)
  419d7f:	movups (%r15),%xmm1
  419d83:	movups %xmm1,0x58(%rsp)
  419d88:	movq   $0x1,0x8(%rsp)
  419d91:	movups %xmm0,0x10(%rsp)
  419d96:	mov    0x50(%rsp),%rax
  419d9b:	mov    %rax,0x20(%rsp)
  419da0:	mov    0x58(%rsp),%rax
  419da5:	mov    %rax,0x28(%rsp)
  419daa:	mov    0x60(%rsp),%rax
  419daf:	mov    %rax,0x30(%rsp)
  419db4:	mov    0x68(%rsp),%rax
  419db9:	mov    %rax,0x38(%rsp)
  419dbe:	mov    $0x38,%edi
  419dc3:	call   *0x536f27(%rip)        # 950cf0 <_DYNAMIC+0x228>
  419dc9:	test   %rax,%rax
  419dcc:	je     419e8c <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x3dc>
  419dd2:	mov    %rax,%r15
  419dd5:	mov    0x38(%rsp),%rax
  419dda:	mov    %rax,0x30(%r15)
  419dde:	movups 0x8(%rsp),%xmm0
  419de3:	movups 0x18(%rsp),%xmm1
  419de8:	movups 0x28(%rsp),%xmm2
  419ded:	movups %xmm2,0x20(%r15)
  419df2:	movups %xmm1,0x10(%r15)
  419df7:	movups %xmm0,(%r15)
  419dfb:	add    $0x8,%r15
  419dff:	mov    0x20(%r14),%rdi
  419e03:	lock decq -0x8(%rdi)
  419e08:	jne    419e13 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x363>
  419e0a:	add    $0xfffffffffffffff8,%rdi
  419e0e:	call   429710 <triomphe::arc::Arc<T>::drop_slow>
  419e13:	mov    %r15,0x20(%r14)
  419e17:	xor    %ebp,%ebp
  419e19:	jmp    419e3c <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x38c>
  419e1b:	xor    %ebp,%ebp
  419e1d:	mov    %r14,%rdi
  419e20:	mov    %r12,%rsi
  419e23:	mov    %r15,%rdx
  419e26:	xor    %ecx,%ecx
  419e28:	call   419ab0 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins>
  419e2d:	test   %al,%al
  419e2f:	je     419e3c <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x38c>
  419e31:	mov    %r14,%rdi
  419e34:	call   41a5d0 <rpds::map::red_black_tree_map::Node<K,V,P>::balance>
  419e39:	mov    $0x1,%bpl
  419e3c:	test   %bl,%bl
  419e3e:	je     419e45 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x395>
  419e40:	movb   $0x1,0x28(%r14)
  419e45:	mov    %ebp,%eax
  419e47:	add    $0x78,%rsp
  419e4b:	pop    %rbx
  419e4c:	pop    %r12
  419e4e:	pop    %r13
  419e50:	pop    %r14
  419e52:	pop    %r15
  419e54:	pop    %rbp
  419e55:	ret
  419e56:	mov    $0x8,%edi
  419e5b:	mov    $0x38,%esi
  419e60:	call   *0x536ed2(%rip)        # 950d38 <_DYNAMIC+0x270>
  419e66:	jmp    419e9c <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x3ec>
  419e68:	mov    $0x8,%edi
  419e6d:	mov    $0x38,%esi
  419e72:	call   *0x536ec0(%rip)        # 950d38 <_DYNAMIC+0x270>
  419e78:	jmp    419e9c <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x3ec>
  419e7a:	mov    $0x8,%edi
  419e7f:	mov    $0x38,%esi
  419e84:	call   *0x536eae(%rip)        # 950d38 <_DYNAMIC+0x270>
  419e8a:	jmp    419e9c <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x3ec>
  419e8c:	mov    $0x8,%edi
  419e91:	mov    $0x38,%esi
  419e96:	call   *0x536e9c(%rip)        # 950d38 <_DYNAMIC+0x270>
  419e9c:	ud2
  419e9e:	mov    %rax,%rbx
  419ea1:	jmp    419eb5 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x405>
  419ea3:	mov    %rax,%rbx
  419ea6:	mov    %r13,%rdi
  419ea9:	call   *0x18(%rsp)
  419ead:	mov    0x8(%rsp),%rcx
  419eb2:	mov    %rax,(%rcx)
  419eb5:	mov    (%r15),%rsi
  419eb8:	test   %rsi,%rsi
  419ebb:	je     419ec9 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x419>
  419ebd:	mov    0x8(%r15),%rdi
  419ec1:	xor    %edx,%edx
  419ec3:	call   *0x536e5f(%rip)        # 950d28 <_DYNAMIC+0x260>
  419ec9:	mov    (%r12),%rsi
  419ecd:	test   %rsi,%rsi
  419ed0:	je     419f11 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x461>
  419ed2:	mov    0x8(%r12),%rdi
  419ed7:	xor    %edx,%edx
  419ed9:	call   *0x536e49(%rip)        # 950d28 <_DYNAMIC+0x260>
  419edf:	mov    %rbx,%rdi
  419ee2:	call   909540 <_Unwind_Resume@plt>
  419ee7:	call   *0x5373eb(%rip)        # 9512d8 <_DYNAMIC+0x810>
  419eed:	jmp    419f21 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x471>
  419eef:	mov    %rax,%rbx
  419ef2:	lea    0x8(%rsp),%rdi
  419ef7:	call   40b980 <core::ptr::drop_in_place<triomphe::arc::ArcInner<rpds::map::red_black_tree_map::Node<alloc::vec::Vec<u8>,alloc::vec::Vec<u8>,archery::shared_pointer::kind::arct::ArcTK>>>>
  419efc:	jmp    419f11 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x461>
  419efe:	call   *0x5373d4(%rip)        # 9512d8 <_DYNAMIC+0x810>
  419f04:	mov    %rax,%rbx
  419f07:	lea    0x8(%rsp),%rdi
  419f0c:	call   40b980 <core::ptr::drop_in_place<triomphe::arc::ArcInner<rpds::map::red_black_tree_map::Node<alloc::vec::Vec<u8>,alloc::vec::Vec<u8>,archery::shared_pointer::kind::arct::ArcTK>>>>
  419f11:	mov    %rbx,%rdi
  419f14:	call   909540 <_Unwind_Resume@plt>
  419f19:	call   *0x5373b9(%rip)        # 9512d8 <_DYNAMIC+0x810>
  419f1f:	jmp    419f21 <rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins+0x471>
  419f21:	mov    %rax,%rbx
  419f24:	lea    0x8(%rsp),%rdi
  419f29:	call   409590 <core::ptr::drop_in_place<triomphe::arc::ArcInner<rpds::map::entry::Entry<alloc::vec::Vec<u8>,alloc::vec::Vec<u8>>>>>
  419f2e:	mov    %rbx,%rdi
  419f31:	call   909540 <_Unwind_Resume@plt>
